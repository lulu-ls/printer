mod converter;
mod pdf;
mod printer;
mod logger;
mod html_webview;
mod downloader;

use log::{error, info};
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use tauri::menu::{CheckMenuItem, Menu, Submenu};
use tauri::Manager;
use tauri::Emitter;

/// 全局 AppHandle（供 html_webview 调度到主线程使用）
pub(crate) static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

// ── Windows 原生打印机枚举（零进程、瞬时，彻底规避 PowerShell 启动卡顿） ──
#[cfg(target_os = "windows")]
use std::sync::Mutex;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use windows::core::PWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Printing::PRINTER_HANDLE;

#[cfg(target_os = "windows")]
struct PrinterCache {
    at: Instant,
    printers: Vec<(String, bool)>,
}

#[cfg(target_os = "windows")]
static PRINTER_CACHE: OnceLock<Mutex<Option<PrinterCache>>> = OnceLock::new();

/// 带 3 秒 TTL 的缓存：避免频繁打开/收起弹窗时重复枚举打印机。
/// 名称用 EnumPrintersW 一次性枚举；在线状态逐个 OpenPrinter 取最新值
/// （EnumPrinters 返回的 Status 多为缓存态、对离线/网络打印机不可靠，
///  等价于 PowerShell Get-Printer 的 PrinterStatus）。
#[cfg(target_os = "windows")]
fn cached_printers() -> Vec<(String, bool)> {
    let cell = PRINTER_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap();
    let now = Instant::now();
    if let Some(c) = guard.as_ref() {
        if now.duration_since(c.at) < Duration::from_secs(2) {
            return c.printers.clone();
        }
    }
    let names = enum_printer_names();
    let printers: Vec<(String, bool)> = names
        .into_iter()
        .map(|n| {
            let online = printer_online_status(&n);
            (n, online)
        })
        .collect();
    *guard = Some(PrinterCache {
        at: now,
        printers: printers.clone(),
    });
    printers
}

/// 用 EnumPrintersW 一次性拿到全部打印机名称（轻量）。
#[cfg(target_os = "windows")]
fn enum_printer_names() -> Vec<String> {
    use windows::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_2W,
    };
    use windows::core::PCWSTR;

    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed: u32 = 0;
    let mut returned: u32 = 0;
    // 第一次调用：仅获取所需缓冲区大小（pprinterenum 传 None）
    let _ = unsafe {
        EnumPrintersW(flags, PCWSTR::null(), 2, None, &mut needed, &mut returned)
    };
    if needed == 0 {
        return Vec::new();
    }
    let mut buf: Vec<u8> = vec![0u8; needed as usize];
    let res = unsafe {
        EnumPrintersW(
            flags,
            PCWSTR::null(),
            2,
            Some(buf.as_mut_slice()),
            &mut needed,
            &mut returned,
        )
    };
    if res.is_err() {
        return Vec::new();
    }

    let count = returned as usize;
    let arr = buf.as_ptr() as *const PRINTER_INFO_2W;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let info = unsafe { &*arr.add(i) };
        let name = unsafe { pwstr_to_string(info.pPrinterName) };
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// 逐个打开打印机读取真实在线状态（等价于 PowerShell Get-Printer 的 PrinterStatus）。
/// 离线/打不开/缺纸/缺墨/需人工干预/手动"脱机工作"等任意条件满足即判为离线。
#[cfg(target_os = "windows")]
fn printer_online_status(name: &str) -> bool {
    use windows::Win32::Graphics::Printing::{ClosePrinter, OpenPrinterW};
    use windows::core::PCWSTR;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut h = PRINTER_HANDLE::default();
    // 打不开（如连接失败）即视为离线/不可用
    if unsafe { OpenPrinterW(PCWSTR(wide.as_ptr()), &mut h, None) }.is_err() {
        return false;
    }
    let online = unsafe { get_printer_status(h) };
    let _ = unsafe { ClosePrinter(h) };
    online
}

#[cfg(target_os = "windows")]
unsafe fn get_printer_status(h: PRINTER_HANDLE) -> bool {
    use windows::Win32::Graphics::Printing::{
        GetPrinterW, PRINTER_INFO_2W, PRINTER_ATTRIBUTE_WORK_OFFLINE,
        PRINTER_STATUS_DOOR_OPEN, PRINTER_STATUS_ERROR, PRINTER_STATUS_NO_TONER,
        PRINTER_STATUS_NOT_AVAILABLE, PRINTER_STATUS_OFFLINE, PRINTER_STATUS_PAPER_OUT,
        PRINTER_STATUS_USER_INTERVENTION,
    };

    let mut needed: u32 = 0;
    // 第一次调用：仅取所需缓冲区大小
    let _ = GetPrinterW(h, 2, None, &mut needed);
    if needed == 0 {
        return true; // 无法获取，保守视为在线
    }
    let mut buf: Vec<u8> = vec![0u8; needed as usize];
    if GetPrinterW(h, 2, Some(buf.as_mut_slice()), &mut needed).is_err() {
        return true;
    }
    let info = &*(buf.as_ptr() as *const PRINTER_INFO_2W);
    let s = info.Status;
    let attr = info.Attributes;
    let offline = (s & PRINTER_STATUS_OFFLINE) != 0
        || (s & PRINTER_STATUS_ERROR) != 0
        || (s & PRINTER_STATUS_PAPER_OUT) != 0
        || (s & PRINTER_STATUS_NOT_AVAILABLE) != 0
        || (s & PRINTER_STATUS_NO_TONER) != 0
        || (s & PRINTER_STATUS_DOOR_OPEN) != 0
        || (s & PRINTER_STATUS_USER_INTERVENTION) != 0
        || (attr & PRINTER_ATTRIBUTE_WORK_OFFLINE) != 0;
    !offline
}

#[cfg(target_os = "windows")]
unsafe fn pwstr_to_string(p: PWSTR) -> String {
    if p.0.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.0.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(p.0, len);
    String::from_utf16_lossy(slice)
}

#[derive(Serialize, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: String,
    pub ext: String,
    pub path: String,
}

#[derive(Serialize, Clone)]
pub struct PrinterStatusItem {
    pub name: String,
    pub online: bool,
}

#[tauri::command]
#[cfg(target_os = "windows")]
fn list_printers() -> Vec<String> {
    // 原生 Win32 枚举，零进程、瞬时（替代 PowerShell Get-Printer）
    cached_printers().into_iter().map(|(n, _)| n).collect()
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
fn list_printers() -> Vec<String> {
    let output = match Command::new("lpstat").arg("-p").output() {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut printers = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("printer ") {
            if let Some(name) = rest.split_whitespace().next() {
                printers.push(name.to_string());
            }
        }
    }
    printers
}

#[tauri::command]
#[cfg(target_os = "windows")]
fn get_default_printer() -> String {
    // 原生 Win32 GetDefaultPrinterW，零进程、瞬时
    use windows::Win32::Graphics::Printing::GetDefaultPrinterW;
    use windows::core::PWSTR;
    let mut buf = [0u16; 256];
    let mut size: u32 = buf.len() as u32;
    let r = unsafe { GetDefaultPrinterW(Some(PWSTR(buf.as_mut_ptr())), &mut size) };
    if r.as_bool() {
        let len = (size as usize).saturating_sub(1).min(buf.len());
        String::from_utf16_lossy(&buf[..len])
    } else {
        String::new()
    }
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
fn get_default_printer() -> String {
    let output = match Command::new("lpstat").arg("-d").output() {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let s = text.trim();
    if let Some(idx) = s.rfind(": ") {
        return s[idx + 2..].trim().to_string();
    }
    String::new()
}

#[tauri::command]
fn get_file_info(path: String) -> Result<FileInfo, String> {
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| "FILE".to_string());

    let size = metadata.len();
    let size_str = if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
    };

    Ok(FileInfo {
        name,
        size: size_str,
        ext,
        path,
    })
}

/// 统一打印管线：文件 -> PDF（第一层）-> 系统打印 API（第二层）。
/// 全程不打开任何第三方应用窗口。
///
/// - `settings`: 打印设置（份数/颜色/双面/方向），可空
/// - `file_id`: 前端文件唯一 id，用于向后端发打印进度事件
#[tauri::command]
#[allow(unused_variables)]
async fn print_file(
    app: tauri::AppHandle,
    path: String,
    printer_name: String,
    settings: Option<printer::PrintSettings>,
    file_id: Option<u64>,
) -> Result<String, String> {
    info!(target: "print", "print_file: {} -> printer {:?}", path, printer_name);
    let input = Path::new(&path);
    let settings = printer::PrintSettings::from_optional(settings.as_ref());

    // 通知前端：开始转换
    let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "converting" }));

    // Windows 下 PDF 优先走打包的 SumatraPDF sidecar
    #[cfg(target_os = "windows")]
    {
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // PDF：尝试 SumatraPDF sidecar（静默、可指定打印机）
        if ext == "pdf" {
            if let Ok(msg) = printer::windows::print_pdf_via_sumatra(&app, input, &printer_name).await {
                let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "sending" }));
                return Ok(msg);
            }
        }

        // 图片：程序内置 GDI 直打（零依赖、静默、可指定打印机）
        if matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp"
        ) {
            // 优先程序内置 GDI 直打（零依赖、静默、可指定打印机）；失败回退通用管线
            if let Ok(msg) = printer::windows::print_image(input, &printer_name) {
                let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "sending" }));
                return Ok(msg);
            }
        }

        // Office 文档：优先用本机已装办公软件（WPS / Office）COM 自动化静默打印
        if matches!(
            ext.as_str(),
            "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx"
        ) {
            match printer::windows::print_office_via_com(input, &printer_name) {
                Ok(msg) => {
                    let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "sending" }));
                    return Ok(msg);
                }
                Err(e) => log::warn!(target: "office", "Office COM 静默打印失败，回退 LibreOffice 管线: {}", e),
            }
        }
    }

    // 1) 优先 LibreOffice：转换为 PDF 后打印
    let lo_pdf = converter::to_pdf(input);
    if let Ok(ref pdf) = lo_pdf {
        // 通知前端：转换完成，开始发送打印任务
        let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "sending" }));
        if let Ok(msg) = printer::print_pdf(pdf, &printer_name, &settings) {
            return Ok(msg);
        }
    }

    // 2) macOS：LO 不可用时，尝试 AppleScript 直接打印
    #[cfg(target_os = "macos")]
    {
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(
            ext.as_str(),
            "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx"
        ) {
            if let Ok(msg) = printer::macos::print_office_via_applescript(input) {
                let _ = app.emit("print-progress", serde_json::json!({ "fileId": file_id, "status": "sending" }));
                return Ok(msg);
            }
        }
    }

    // 3) 最终：返回 LO 转换的错误
    let pdf = lo_pdf.map_err(|e| { error!(target: "print", "转换失败: {} ({})", e, path); e })?;
    printer::print_pdf(&pdf, &printer_name, &settings)
        .map_err(|e| { error!(target: "print", "打印失败: {} ({})", e, printer_name); e })
}

/// 本机是否可用 LibreOffice（Office 文档打印所需）。
#[tauri::command]
fn libreoffice_available() -> bool {
    converter::office::libreoffice_available()
}

/// 本机是否可用办公软件自动化（Windows: COM / macOS: AppleScript），用于 Office 文档静默打印。
/// 优先于 LibreOffice，无需额外下载安装。
#[tauri::command]
fn office_automation_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        let ok = printer::windows::detect_office_com("docx").is_some();
        log::info!(target: "office", "office_automation_available -> {}", ok);
        ok
    }
    #[cfg(target_os = "macos")]
    {
        let ok = printer::macos::macos_office_available();
        log::info!(target: "office", "macos_office_available -> {}", ok);
        ok
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

/// 仅把文件转换为 PDF 并输出到临时目录，返回 PDF 路径（不实际打印）。
/// 用于没有打印机时预览生成的 PDF。
#[tauri::command]
fn build_pdf(path: String) -> Result<String, String> {
    info!(target: "build", "build_pdf: {}", path);
    let input = Path::new(&path);

    // 1) 优先 LibreOffice 转换
    let lo_result = converter::to_pdf(input);
    if let Ok(pdf) = &lo_result {
        return Ok(pdf.to_string_lossy().to_string());
    }

    // 2) macOS：LO 不可用时尝试 AppleScript（MS Office 另存为 PDF）
    #[cfg(target_os = "macos")]
    {
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx") {
            let tmp = converter::temp_dir();
            if let Ok(pdf_path) = printer::macos::office_to_pdf_via_applescript(input, &tmp) {
                return Ok(pdf_path.to_string_lossy().to_string());
            }
        }
    }

    // 3) 返回原始 LO 错误
    lo_result.map(|p| p.to_string_lossy().to_string())
        .map_err(|e| { error!(target: "build", "build_pdf 失败: {} ({})", e, path); e })
}

/// 该文件打印是否需要 LibreOffice。
#[tauri::command]
fn needs_libreoffice(path: String) -> bool {
    converter::office::requires_libreoffice(Path::new(&path))
}

/// 用系统默认方式打开一个 URL（用于引导下载 LibreOffice）。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .status()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", &url])
            .status()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Command::new("xdg-open")
            .arg(&url)
            .status()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// 返回当前运行平台："windows" | "macos" | "linux"，供前端区分窗口样式。
#[tauri::command]
fn platform() -> String {
    #[cfg(target_os = "windows")]
    {
        "windows".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "macos".to_string()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "linux".to_string()
    }
}

/// 检测指定打印机是否在线/空闲。
/// Windows：原生 Win32 枚举（零进程）；macOS/Linux：lpstat -p 包含 "is idle" / "is printing"。
#[tauri::command]
fn printer_online(name: String) -> bool {
    #[cfg(target_os = "windows")]
    {
        cached_printers()
            .into_iter()
            .find(|(n, _)| n == &name)
            .map(|(_, online)| online)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let out = Command::new("lpstat")
            .arg("-p")
            .arg(&name)
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).to_string())
                } else {
                    None
                }
            });
        out.map_or(false, |s| {
            s.contains("is idle") || s.contains("is printing")
        })
    }
}

/// 批量查询全部打印机的在线状态（Windows 原生枚举，零进程；macOS/Linux 用 lpstat）。
#[tauri::command]
fn printers_status() -> Vec<PrinterStatusItem> {
    #[cfg(target_os = "windows")]
    {
        cached_printers()
            .into_iter()
            .map(|(name, online)| PrinterStatusItem { name, online })
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("lpstat")
            .arg("-p")
            .output()
            .ok()
            .and_then(|o| if o.status.success() { Some(o.stdout) } else { None });
        let output = match output {
            Some(b) => String::from_utf8_lossy(&b).to_string(),
            None => return vec![],
        };
        let mut statuses: Vec<PrinterStatusItem> = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("printer ") {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() >= 2 {
                    let name = parts[0].to_string();
                    let state = parts[1];
                    let online = state.contains("is idle") || state.contains("is printing");
                    statuses.push(PrinterStatusItem { name, online });
                }
            }
        }
        statuses
    }
}

/// 读取文件文本内容并内联外部资源（CSS 内联 + 相对路径转绝对 file:// 路径）
#[tauri::command]
fn read_html_file(path: String) -> Result<String, String> {
    use std::path::Path;

    let p = Path::new(&path);
    let dir = p.parent().unwrap_or(Path::new("/"));
    let content = std::fs::read_to_string(p).map_err(|e| format!("读取失败: {}", e))?;

    // 手动扫描 HTML，内联 CSS 和解析图片/资源路径
    let result = inline_resources(&content, dir);
    Ok(result)
}

fn inline_resources(html: &str, base_dir: &Path) -> String {
    let mut out = String::with_capacity(html.len() + 4096);
    let mut remaining = html;

    while !remaining.is_empty() {
        // 找 <link 或 <img 开头
        let find_link = remaining.find("<link");
        let find_img  = remaining.find("<img");
        let find_script = remaining.find("<script");

        let earliest = [find_link, find_img, find_script]
            .iter().filter_map(|&x| x).min();

        match earliest {
            None => { out.push_str(remaining); break; }
            Some(pos) => {
                out.push_str(&remaining[..pos]);
                remaining = &remaining[pos..];
                let tag_end = remaining.find('>').unwrap_or(remaining.len() - 1);
                let tag = &remaining[..=tag_end];
                remaining = &remaining[tag_end + 1..];
                let tag_lower = tag.to_lowercase();

                // <link rel="stylesheet" href="..."> → 内联
                if tag_lower.starts_with("<link") && tag_lower.contains("stylesheet") {
                    if let Some(href) = extract_attr_value(tag, "href") {
                        let css_path = base_dir.join(&href);
                        if let Ok(css) = std::fs::read_to_string(&css_path) {
                            let resolved = resolve_css_urls(&css, base_dir);
                            out.push_str("<style>\n");
                            out.push_str(&resolved);
                            out.push_str("\n</style>\n");
                            continue;
                        }
                    }
                }

                // <img src="..."> 相对路径 → 绝对路径
                if tag_lower.starts_with("<img") {
                    if let Some(src) = extract_attr_value(tag, "src") {
                        if !src.starts_with("http://") && !src.starts_with("https://")
                            && !src.starts_with("file://") && !src.starts_with("data:")
                            && !src.starts_with('/')
                        {
                            let abs = format!("file://{}/{}", base_dir.display(), src);
                            let new_tag = tag.replace(&format!("src=\"{}\"", &src),
                                &format!("src=\"{}\"", &abs));
                            out.push_str(&new_tag);
                            continue;
                        }
                    }
                }

                out.push_str(tag);
            }
        }
    }
    out
}

fn extract_attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pattern = format!("{}=\"", attr.to_lowercase());
    if let Some(start) = lower.find(&pattern) {
        let val_start = start + pattern.len();
        let tag_chars: Vec<char> = tag.chars().collect();
        let mut val = String::new();
        let mut i = val_start;
        while i < tag_chars.len() && tag_chars[i] != '"' {
            val.push(tag_chars[i]);
            i += 1;
        }
        return Some(val);
    }
    // 单引号版本
    let pattern = format!("{}='", attr.to_lowercase());
    if let Some(start) = lower.find(&pattern) {
        let val_start = start + pattern.len();
        let tag_chars: Vec<char> = tag.chars().collect();
        let mut val = String::new();
        let mut i = val_start;
        while i < tag_chars.len() && tag_chars[i] != '\'' {
            val.push(tag_chars[i]);
            i += 1;
        }
        return Some(val);
    }
    None
}

fn resolve_css_urls(css: &str, base_dir: &Path) -> String {
    // 把 CSS 中 url(...) 的相对路径转绝对路径
    let mut result = String::with_capacity(css.len() + 512);
    let mut remaining = css;

    while let Some(pos) = remaining.find("url(") {
        result.push_str(&remaining[..pos]);
        remaining = &remaining[pos + 4..]; // skip "url("

        // 寻找匹配的 )
        let mut depth = 1u32;
        let mut end = 0;
        let chars: Vec<char> = remaining.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if c == '(' { depth += 1; }
            else if c == ')' { depth -= 1; if depth == 0 { end = i; break; } }
        }

        let url_expr = &remaining[..end];
        remaining = &remaining[end + 1..];

        // 提取 URL（去除引号）
        let url_value = url_expr.trim().trim_matches('"').trim_matches('\'');

        if !url_value.starts_with("http://") && !url_value.starts_with("https://")
            && !url_value.starts_with("file://") && !url_value.starts_with("data:")
            && !url_value.starts_with('/') && !url_value.starts_with('#')
        {
            let abs = format!("file://{}/{}", base_dir.display(), url_value);
            result.push_str(&format!("url({})", &abs));
        } else {
            result.push_str(&format!("url({})", url_expr));
        }
    }
    result.push_str(remaining);
    result
}

/// 保存前端传来的 PDF 字节到临时文件
#[tauri::command]
fn save_pdf(data: Vec<u8>, filename: String) -> Result<String, String> {
    let tmp = std::env::temp_dir().join("printer_assistant").join(&filename);
    if let Some(parent) = tmp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&tmp, &data).map_err(|e| format!("写入失败: {}", e))?;
    Ok(tmp.to_string_lossy().to_string())
}

/// 取消一个已提交的打印任务（macOS CUPS）
#[tauri::command]
#[allow(unused_variables)]
fn cancel_print_job(job_id: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        printer::macos::cancel_print_job(&job_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = job_id;
        Ok(())
    }
}

/// 清理转换临时目录里的旧文件。
/// `older_than_days`: 只删除超过该天数的文件；传 0 表示清空所有。
/// 返回删除的文件数量。
#[tauri::command]
fn clean_temp_files(older_than_days: Option<u64>) -> Result<u64, String> {
    let days = older_than_days.unwrap_or(1);
    let dir = converter::temp_dir();
    if !dir.exists() {
        return Ok(0);
    }

    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days.saturating_mul(86400)))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut removed: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(cutoff);
            if modified <= cutoff {
                if std::fs::remove_file(&p).is_ok() {
                    removed += 1;
                }
            }
        }
    }

    info!(target: "clean", "清理临时文件: 删除 {} 个 (>{:?} 天)", removed, days);
    Ok(removed)
}
#[tauri::command]
fn get_language(app: tauri::AppHandle) -> String {
    load_lang(&app)
}

/// 前端（JavaScript）把日志/报错写入同一份日志文件，便于统一排查。
/// level: "error" | "warn" | "info" | "debug" | "trace"
#[tauri::command]
fn log_message(level: String, msg: String) {
    match level.as_str() {
        "error" => error!(target: "frontend", "{}", msg),
        "warn"  => log::warn!(target: "frontend", "{}", msg),
        "debug" => log::debug!(target: "frontend", "{}", msg),
        "trace" => log::trace!(target: "frontend", "{}", msg),
        _       => info!(target: "frontend", "{}", msg),
    }
}

/// 是否启用 Demo 模式（环境变量 DEMO=true，用于录演示视频）。
#[tauri::command]
fn is_demo() -> bool {
    std::env::var("DEMO").map(|v| v == "1" || v == "true").unwrap_or(false)
}

// ── 语言偏好的持久化（存于应用配置目录的 lang.txt） ──────────
fn lang_file_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        return Some(dir.join("lang.txt"));
    }
    None
}

fn load_lang(app: &tauri::AppHandle) -> String {
    if let Some(p) = lang_file_path(app) {
        if let Ok(s) = std::fs::read_to_string(&p) {
            let s = s.trim().to_string();
            if s == "en" || s == "zh" {
                return s;
            }
        }
    }
    "zh".to_string()
}

fn save_lang(app: &tauri::AppHandle, lang: &str) {
    if let Some(p) = lang_file_path(app) {
        let _ = std::fs::write(&p, lang);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            list_printers,
            get_default_printer,
            get_file_info,
            print_file,
            libreoffice_available,
            office_automation_available,
            needs_libreoffice,
            open_url,
            build_pdf,
            get_language,
            log_message,
            platform,
            printer_online,
            printers_status,
            is_demo,
            read_html_file,
            save_pdf,
            clean_temp_files,
            cancel_print_job,
            downloader::download_libreoffice,
        ])
        .setup(|app| {
            // 初始化文件日志（500KB 上限，超出自动截断旧内容）
            crate::logger::init(app.handle());
            let _ = crate::APP_HANDLE.set(app.handle().clone());

            // Windows 不使用 Overlay 标题栏（否则会多出一块深色标题区），改用原生标题栏
            #[cfg(target_os = "windows")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_title_bar_style(tauri::TitleBarStyle::Visible);
                }
            }

            // panic 钩子：把崩溃信息写入日志
            std::panic::set_hook(Box::new(|info| {
                let loc = info.location().map(|l| l.to_string()).unwrap_or_default();
                error!(target: "panic", "线程 panic @ {}: {}", loc, info);
            }));

            // 初始化常驻 PDF 打印引擎（macOS 创建隐藏 WKWebView）
            info!(target: "app", "初始化打印引擎...");
            match crate::html_webview::init_print_engine() {
                Ok(_) => info!(target: "app", "打印引擎初始化成功"),
                Err(e) => error!(target: "app", "打印引擎初始化失败: {}", e),
            }

            // 启动时清理 1 天前的转换临时文件
            {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    let _ = crate::clean_temp_files(Some(1));
                    let _ = app.emit("temp-cleaned", serde_json::json!({}));
                });
            }

            info!(target: "app", "应用启动");
            let handle = app.handle();
            let lang = load_lang(handle);

            // 语言子菜单：简体中文 / English
            let lang_zh = CheckMenuItem::with_id(handle, "lang-zh", "简体中文", true, lang == "zh", None::<&str>)
                .expect("failed to create lang-zh menu item");
            let lang_en = CheckMenuItem::with_id(handle, "lang-en", "English", true, lang == "en", None::<&str>)
                .expect("failed to create lang-en menu item");
            let sub_label = if lang == "zh" { "语言" } else { "Language" };
            let lang_menu = Submenu::with_id(handle, "lang-menu", sub_label, true)
                .expect("failed to create lang submenu");
            lang_menu
                .append_items(&[&lang_zh, &lang_en])
                .expect("failed to append lang menu items");

            // 追加到默认原生菜单（App / File / Edit / View / Window / Help 之后）
            match Menu::default(handle) {
                Ok(menu) => {
                    let _ = menu.append(&lang_menu);
                    let _ = app.set_menu(menu);
                }
                Err(_) => {
                    let menu = Menu::with_items(handle, &[&lang_menu])
                        .expect("failed to create menu");
                    let _ = app.set_menu(menu);
                }
            }

            Ok(())
        });

    // 菜单选择语言 -> 持久化 + 通知前端动态切换
    let builder = builder.on_menu_event(|app, event| {
        let id = event.id().0.clone();
        let lang = match id.as_str() {
            "lang-zh" => "zh",
            "lang-en" => "en",
            _ => return,
        };
        save_lang(app, lang);

        // 更新勾选状态（语言项位于 "lang-menu" 子菜单内，需从子菜单中查找）
        if let Some(menu) = app.menu() {
            if let Some(sub_item) = menu.get("lang-menu") {
                if let Some(sub) = sub_item.as_submenu() {
                    // 子菜单标题随语言变化
                    let _ = sub.set_text(if lang == "zh" { "语言" } else { "Language" });
                    if let Some(item) = sub.get("lang-zh") {
                        if let Some(check) = item.as_check_menuitem() {
                            let _ = check.set_checked(lang == "zh");
                        }
                    }
                    if let Some(item) = sub.get("lang-en") {
                        if let Some(check) = item.as_check_menuitem() {
                            let _ = check.set_checked(lang == "en");
                        }
                    }
                }
            }
        }

        let _ = app.emit("language-changed", lang);
    });

    builder
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {});
}
