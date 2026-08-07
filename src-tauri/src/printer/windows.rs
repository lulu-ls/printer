// Windows 打印：SumatraPDF sidecar（推荐）+ MuPDF `mutool`（可选）+ Shell 回退 +
//            GDI 直打图片 + COM 自动化打印 Office
//
// PDF：优先用打包的 SumatraPDF sidecar 静默打印（可指定打印机）；
//      未打包时回退 MuPDF `mutool draw -o "printer:<打印机名>"`；
//      均不可用时回退系统默认程序 Shell 打印（使用默认打印机）。
// 图片：GDI 直打（零依赖，静默打印）。
// Office：COM 自动化（WPS / Microsoft Office）静默打印，失败回退 LibreOffice 管线。

use std::path::Path;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 在 Windows 上隐藏子进程的控制台窗口（避免打印时弹出黑框）。
/// `CREATE_NO_WINDOW`(0x08000000)：不为子进程创建控制台窗口。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 为命令设置隐藏控制台窗口标志（Windows）；其它平台原样返回。
#[cfg(target_os = "windows")]
fn hide_console(mut cmd: Command) -> Command {
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn hide_console(cmd: Command) -> Command {
    cmd
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn print_via_mutool(path: &Path, printer: &str, settings: &crate::printer::PrintSettings) -> Result<String, String> {
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();

    let output_target = format!("printer:{}", printer);

    let mut cmd = Command::new("mutool");
    cmd.args(["draw", "-o", &output_target]);
    // 份数（mutool 通过 -p 或 print options 支持有限，这里记录日志便于排查）
    if settings.copies > 1 {
        log::info!(target: "print", "mutool 打印份数: {}", settings.copies);
    }
    cmd.arg(&path_str);

    let status = cmd.status();

    match status {
        Ok(s) if s.success() => Ok("ok".into()),
        // mutool 未安装或打印失败：回退到系统默认程序 Shell 打印（不指定打印机，用默认打印机）
        _ => {
            log::warn!(
                target: "print",
                "mutool 不可用或返回非 0，回退 Shell 打印: {:?}",
                status
            );
            print_via_shell_print(path)
        }
    }
}

/// 用系统默认关联程序以 `Print` 动词打印文件（回退方案，不依赖 mutool / SumatraPDF）。
/// 注意：Shell `Print` 动词只能使用系统默认打印机，无法指定目标打印机。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn print_via_shell_print(path: &Path) -> Result<String, String> {
    let path_str = path.to_string_lossy().replace('\'', "''");
    let script = format!("Start-Process -FilePath '{}' -Verb Print", path_str);
    let status = hide_console(Command::new("powershell"))
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| format!("无法启动打印进程: {}", e))?;
    if status.success() {
        Ok("已提交打印任务（PDF 通过系统默认程序打印，未指定打印机）".into())
    } else {
        Err(format!("打印失败，退出码 {}", status.code().unwrap_or(-1)))
    }
}

/// 检测本机可用的办公软件 COM 自动化对象（WPS / Microsoft Office）。
/// 返回 Some(progid)，如 "wps.application" / "Word.Application" 等；检测不到返回 None。
/// 该探测完全静默：只尝试 `New-Object -ComObject`，不弹出任何窗口、不启动可见界面。
///
/// 注意：本函数只判断「能否创建 COM 对象」，不代表一定能静默打印成功；
/// 真正的打印在 `print_office_via_com` 中执行，失败会回退 LibreOffice 管线。
/// 32 位 WPS 只注册了 32 位 COM（progid 形如 Kwps/Ket/Kwpp.Application），
/// 而本程序多为 64 位，默认 64 位 powershell 看不到这些 32 位 COM 对象。
/// 因此所有 Office COM 自动化都改用 32 位 SysWOW64 的 powershell 来执行。
#[cfg(target_os = "windows")]
const POWERSHELL_32: &str = "C:\\Windows\\SysWOW64\\WindowsPowerShell\\v1.0\\powershell.exe";

#[cfg(target_os = "windows")]
pub fn detect_office_com(ext: &str) -> Option<String> {
    let candidates: &[&str] = match ext.to_lowercase().as_str() {
        "doc" | "docx" => &[
            "Kwps.Application", "wps.application", "Word.Application",
        ],
        "xls" | "xlsx" => &[
            "Ket.Application", "et.application", "Excel.Application",
        ],
        "ppt" | "pptx" => &[
            "Kwpp.Application", "wpp.application", "PowerPoint.Application",
        ],
        _ => return None,
    };
    // 注意：不要使用 @(...) 数组字面量 + foreach，因为 -Command 以单行字符串传入时
    // 换行解析不稳，会触发 ParserError。这里改为逐个 try 的顺序尝试，语句间用分号分隔。
    let mut script = String::new();
    for c in candidates {
        script.push_str(&format!(
            "try {{ $o = New-Object -ComObject '{c}'; [void][System.Runtime.Interopservices.Marshal]::ReleaseComObject($o); Write-Output '{c}'; exit }} catch {{ }}; "
        ));
    }
    let out = hide_console(Command::new(POWERSHELL_32))
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;
    let progid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if progid.is_empty() {
        log::info!(target: "office", "detect_office_com({}): 未检测到可用办公软件 COM", ext);
        None
    } else {
        log::info!(target: "office", "detect_office_com({}): 检测到 {}", ext, progid);
        Some(progid)
    }
}

/// 用本机已装的办公软件（WPS / MS Office）COM 自动化，把 Office 文档静默打印到指定打印机。
/// - 不弹出任何窗口（Visible=$false），不弹保存提示（只读打开 + 关闭时不保存）。
/// - `printer_name` 为空时打印到系统默认打印机；非空时尝试设置 ActivePrinter。
/// - 失败（COM 不可用 / 打印异常）返回 Err，由上层回退 LibreOffice 管线。
#[cfg(target_os = "windows")]
pub fn print_office_via_com(input: &Path, printer_name: &str) -> Result<String, String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let progid = detect_office_com(&ext).ok_or_else(|| {
        "未检测到可用的办公软件（WPS / Office），无法静默打印该 Office 文档".to_string()
    })?;

    // 规范化路径：PowerShell 单引号字符串内只需把 ' 转义为 ''；反斜杠在单引号串中无特殊含义
    let path = input
        .canonicalize()
        .map_err(|e| format!("无法解析文件路径: {}", e))?
        .to_string_lossy()
        .replace('\'', "''");
    let printer = printer_name.replace('\'', "''");

    let kind = match ext.as_str() {
        "doc" | "docx" => "word",
        "xls" | "xlsx" => "excel",
        "ppt" | "pptx" => "powerpoint",
        _ => return Err("不支持的 Office 文件类型".into()),
    };

    let mut script = String::new();
    script.push_str("$ErrorActionPreference = 'Stop'\n");
    script.push_str(&format!("$progid = '{}'\n", progid));
    script.push_str(&format!("$path = '{}'\n", path));
    script.push_str(&format!("$printer = '{}'\n", printer));
    script.push_str(&format!("$kind = '{}'\n", kind));

    // 设置目标打印机（ActivePrinter 格式：\"Name on Port:\"；获取不到端口则保持默认打印机）
    script.push_str(
        "if ($printer -ne '') {\n  try {\n    $p = Get-CimInstance -ClassName Win32_Printer | Where-Object { $_.Name -eq $printer } | Select-Object -First 1\n    if ($p -and $p.PortName) { $activePrinter = ($p.Name + ' on ' + $p.PortName) }\n  } catch { }\n}\n",
    );

    // 用 try/catch/finally 包裹整个自动化流程：
    // - 避免 Quit / ReleaseComObject 等收尾语句偶发抛错，把「其实已打印成功」误判为失败；
    // - 真正打印异常时把异常信息写入 stderr，便于排查（如 WPS 演示 PrintOut 行为差异）。
    script.push_str("try {\n");
    script.push_str("  $app = New-Object -ComObject $progid\n");
    // WPS 演示(Kwpp) 设置 Visible=$false 会抛 E_FAIL 并使 COM 对象进入故障态，导致后续 Open 也失败；
    // 因此 PPT 不设置 Visible（保留默认可见窗口），其余类型静默隐藏窗口。
    if kind != "powerpoint" {
        script.push_str("  try { $app.Visible = $false } catch { }\n");
    }
    script.push_str("  if ($activePrinter) { try { $app.ActivePrinter = $activePrinter } catch { } }\n");

    match kind {
        "word" => {
            script.push_str("  $doc = $app.Documents.Open($path, $false, $true, $false)\n");
            script.push_str("  $doc.PrintOut()\n");
            script.push_str("  Start-Sleep -Milliseconds 800\n");
            script.push_str("  $doc.Close([ref]0)\n");
        }
        "excel" => {
            script.push_str("  $wb = $app.Workbooks.Open($path, $false, $true)\n");
            script.push_str("  $wb.PrintOut()\n");
            script.push_str("  Start-Sleep -Milliseconds 800\n");
            script.push_str("  $wb.Close([ref]$false)\n");
        }
        "powerpoint" => {
            // WPS 演示 / PowerPoint 的 Open 签名与 Microsoft 不一致，且部分参数组合会抛 E_FAIL；
            // 这里按顺序尝试多种签名，取第一个成功的，提升兼容性。
            script.push_str("  $pres = $null\n");
            script.push_str("  foreach ($oa in @('s','ro','row','w')) {\n");
            script.push_str("    try {\n");
            script.push_str("      if ($oa -eq 's') { $pres = $app.Presentations.Open($path) }\n");
            script.push_str("      elseif ($oa -eq 'ro') { $pres = $app.Presentations.Open($path, $true) }\n");
            script.push_str("      elseif ($oa -eq 'row') { $pres = $app.Presentations.Open($path, $true, $false, $true) }\n");
            script.push_str("      elseif ($oa -eq 'w') { $pres = $app.Presentations.Open($path, $false, $false, $true) }\n");
            script.push_str("      break\n");
            script.push_str("    } catch { }\n");
            script.push_str("  }\n");
            script.push_str("  if (-not $pres) { throw 'WPS 演示无法打开文件（Open 失败）' }\n");
            script.push_str("  $pres.PrintOut()\n");
            script.push_str("  Start-Sleep -Milliseconds 1500\n");
            script.push_str("  try { $pres.Close() } catch { }\n");
        }
        _ => {}
    }

    script.push_str("  Write-Output 'OK'\n");
    script.push_str("  exit 0\n");
    script.push_str("} catch {\n");
    script.push_str("  Write-Error $_.Exception.Message\n");
    script.push_str("  exit 1\n");
    script.push_str("} finally {\n");
    script.push_str("  try { $app.Quit() } catch { }\n");
    script.push_str("  try { [void][System.Runtime.Interopservices.Marshal]::ReleaseComObject($app) } catch { }\n");
    script.push_str("}\n");

    let out = hide_console(Command::new(POWERSHELL_32))
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| format!("无法启动 PowerShell 自动化: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() && stdout.trim() == "OK" {
        Ok("已提交打印任务（办公软件静默打印）".to_string())
    } else {
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().lines().next().unwrap_or("未知错误").to_string()
        } else {
            stdout.trim().to_string()
        };
        Err(format!("办公软件静默打印失败: {}", detail))
    }
}

/// 用 Windows GDI 把图片直接送到打印机（静默、可指定打印机）。
#[cfg(target_os = "windows")]
fn print_image_gdi(input: &Path, printer_name: &str) -> Result<String, String> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::core::{PCWSTR, PWSTR, HSTRING};
    use windows::Win32::Graphics::Gdi::{
        CreateDCW, DeleteDC, GetDeviceCaps, PHYSICALHEIGHT, PHYSICALWIDTH, SetMapMode,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, MM_TEXT, SRCCOPY,
        StretchDIBits,
    };
    use windows::Win32::Graphics::Printing::GetDefaultPrinterW;
    use windows::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use image::GenericImageView;

    // 1) 解码图片（程序内置 image crate，零外部依赖）
    let img = image::open(input).map_err(|e| format!("无法解码图片: {}", e))?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("图片尺寸无效".into());
    }
    let rgb = img.to_rgb8();
    let (w, h) = (w as i32, h as i32);

    // 24-bit DIB 要求每行按 4 字节对齐（stride）；DIB 像素为 BGR 顺序（非 RGB）。
    let stride = ((w as usize * 3 + 3) & !3) as usize;
    let mut pixels = vec![0u8; stride * h as usize];
    let src = rgb.as_raw();
    let row_bytes = w as usize * 3;
    for y in 0..h as usize {
        let dst = &mut pixels[y * stride..y * stride + row_bytes];
        let s = &src[y * row_bytes..y * row_bytes + row_bytes];
        // RGB -> BGR
        for x in 0..w as usize {
            dst[x * 3] = s[x * 3 + 2];
            dst[x * 3 + 1] = s[x * 3 + 1];
            dst[x * 3 + 2] = s[x * 3];
        }
    }

    // 2) 确定目标打印机
    let printer: HSTRING = if printer_name.is_empty() {
        let mut buf = [0u16; 256];
        let mut size: u32 = buf.len() as u32;
        let r = unsafe { GetDefaultPrinterW(Some(PWSTR::from_raw(buf.as_mut_ptr())), &mut size) };
        if !r.as_bool() {
            return Err("无法获取默认打印机".into());
        }
        let len = (size as usize).min(buf.len());
        HSTRING::from_wide(&buf[..len])
    } else {
        HSTRING::from(printer_name)
    };

    // 3) 连接打印机，得到打印机 DC
    let hdc = unsafe {
        CreateDCW(
            windows::core::w!("WINSPOOL"),
            PCWSTR::from_raw(printer.as_ptr()),
            None,
            None,
        )
    };
    if hdc.is_invalid() {
        return Err("无法连接打印机（CreateDC 失败）".into());
    }

    // 4) 取得物理页面尺寸（设备像素），用于等比适配
    let phw = unsafe { GetDeviceCaps(Some(hdc), PHYSICALWIDTH) };
    let phh = unsafe { GetDeviceCaps(Some(hdc), PHYSICALHEIGHT) };
    unsafe { SetMapMode(hdc, MM_TEXT) };

    // 5) 组织 BITMAPINFO（top-down 24-bit，BGR）
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // 负数 = 自上而下
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 24;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    // 6) 等比居中适配
    let scale = if phw > 0 && phh > 0 {
        ((phw as f64) / (w as f64)).min((phh as f64) / (h as f64))
    } else {
        1.0
    };
    let dw = (w as f64 * scale) as i32;
    let dh = (h as f64 * scale) as i32;
    let dx = (phw - dw) / 2;
    let dy = (phh - dh) / 2;

    let doc_name: HSTRING = HSTRING::from("PrinterAssistant");
    let docinfo = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR::from_raw(doc_name.as_ptr()),
        lpszOutput: PCWSTR::null(),
        lpszDatatype: PCWSTR::null(),
        fwType: 0,
    };

    let result = unsafe {
        let started = StartDocW(hdc, &docinfo);
        if started <= 0 {
            Err("StartDoc 失败".into())
        } else if StartPage(hdc) <= 0 {
            EndDoc(hdc);
            Err("StartPage 失败".into())
        } else {
            let copied = StretchDIBits(
                hdc,
                dx,
                dy,
                dw,
                dh,
                0,
                0,
                w,
                h,
                Some(pixels.as_ptr() as *const c_void),
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            EndPage(hdc);
            EndDoc(hdc);
            if copied != 0 {
                Ok(())
            } else {
                Err("StretchDIBits 绘制失败".into())
            }
        }
    };

    unsafe { let _ = DeleteDC(hdc); }

    match result {
        Ok(()) => Ok("已提交打印任务（内置 GDI 直打）".to_string()),
        Err(e) => Err(e),
    }
}

/// 用打包的 SumatraPDF sidecar 静默打印 PDF（可指定打印机）。
/// 需要在运行前通过 `npm run bundle:sumatra` 部署对应架构的 binary 到 src-tauri/binaries/。
#[cfg(target_os = "windows")]
pub async fn print_pdf_via_sumatra(app: &tauri::AppHandle, path: &Path, printer_name: &str) -> Result<String, String> {
    use tauri_plugin_shell::process::CommandEvent;
    use tauri_plugin_shell::ShellExt;

    let pdf_path = path.to_string_lossy().to_string();

    let sidecar = app.shell().sidecar("sumatrapdf").map_err(|e| {
        log::warn!(target: "print", "SumatraPDF sidecar 不可用: {}", e);
        "SumatraPDF 未打包进应用".to_string()
    })?;

    let cmd = if !printer_name.is_empty() {
        sidecar.args(["-print-to", printer_name, "-silent", pdf_path.as_str()])
    } else {
        sidecar.args(["-silent", pdf_path.as_str()])
    };

    let (mut rx, child) = cmd.spawn().map_err(|e| {
        log::warn!(target: "print", "SumatraPDF 启动失败: {}", e);
        format!("SumatraPDF 启动失败: {}", e)
    })?;

    loop {
        match rx.recv().await {
            Some(CommandEvent::Terminated(payload)) => {
                drop(child);
                return match payload.code {
                    Some(0) => Ok("已提交打印任务（SumatraPDF 静默打印）".to_string()),
                    Some(c) => Err(format!("SumatraPDF 打印失败，退出码 {}", c)),
                    None => Err("SumatraPDF 打印任务无退出状态".to_string()),
                };
            }
            Some(_) => continue,
            None => break,
        }
    }
    drop(child);
    Err("SumatraPDF 打印通道异常关闭".to_string())
}

/// GDI 直打图片（零依赖，静默打印）；失败回退通用管线。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn print_image(input: &Path, printer_name: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    { print_image_gdi(input, printer_name) }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (input, printer_name);
        Err("GDI 直打暂未实现，请使用通用管线".into())
    }
}
