// macOS 打印：CUPS `lp` + AppleScript 打印 Office 文档
//
// 系统原生打印管线，稳定且不会弹出任何第三方窗口（不使用 Preview / osascript）。
// Office 文档（doc/docx/xls/xlsx/ppt/pptx）优先通过 AppleScript 调用
// Microsoft Office 直接打印，避免依赖 LibreOffice。

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn print_via_lp(path: &Path, printer: &str) -> Result<String, String> {
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();

    let mut cmd = Command::new("lp");
    if !printer.is_empty() {
        cmd.arg("-d").arg(printer);
    }
    cmd.arg(&path_str);

    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok("ok".into())
}

// WPS: macOS WPS Office AppleScript 适配（预留，当前版本 WPS AppleScript 接口不完善）
// #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
// fn wps_available() -> bool { ... }

/// 检测 macOS 上可用的办公软件（目前仅 Microsoft Office）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_office_available() -> bool {
    // 检测 Microsoft Office
    let ms_paths = [
        "/Applications/Microsoft Word.app",
        "/Applications/Microsoft Word.app/Contents/MacOS/Microsoft Word",
    ];
    for p in &ms_paths {
        let exists = std::path::Path::new(p).exists();
        log::info!(target: "office", "检查路径: {} -> {}", p, exists);
        if exists {
            log::info!(target: "office", "办公软件检测: Microsoft Office 可用");
            return true;
        }
    }

    // mdfind 兜底
    Command::new("mdfind")
        .arg("kMDItemFSName == 'Microsoft Word.app'")
        .output()
        .ok()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.trim().contains("Microsoft Word")
        })
        .unwrap_or(false)
}

/// 执行一次 AppleScript 静默打印
fn try_applescript_print(app_name: &str, obj_name: &str, path_str: &str, use_posix: bool) -> Result<String, String> {
    let open_cmd = if use_posix {
        format!(r#"open POSIX file "{}""#, path_str)
    } else {
        format!(r#"open "{}""#, path_str)
    };
    let script = format!(
        r#"tell application "{}"
    with timeout of 30 seconds
        {}
        print out active {}
        close active {} saving no
    end timeout
end tell"#,
        app_name, open_cmd, obj_name, obj_name
    );

    log::info!(target: "office", "AppleScript 打印: {} via {}", path_str, app_name);

    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("执行 osascript 失败: {}", e))?;

    if output.status.success() {
        log::info!(target: "office", "AppleScript 打印成功: {} via {}", path_str, app_name);
        Ok(format!("{} 已通过 {} 打印", path_str, app_name))
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        log::warn!(target: "office", "AppleScript 打印失败 ({}): {}", app_name, err);
        Err(format!("AppleScript 打印失败 ({}): {}", app_name, err))
    }
}

/// 通过 AppleScript 静默打印 Office 文档（不弹出任何窗口）。
/// 依次尝试：WPS → Microsoft Office。返回 Ok 表示已成功提交打印。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn print_office_via_applescript(input: &Path) -> Result<String, String> {
    let abs = std::fs::canonicalize(input).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if !matches!(ext.as_str(), "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx") {
        return Err(format!("不支持的 Office 格式: {}", ext));
    }

    // WPS: 预留 WPS Office 打印适配，当前版本 AppleScript 接口不完善
    // 后续可在此处插入 WPS 尝试逻辑

    // 再试 Microsoft Office
    let (app, obj) = match ext.as_str() {
        "doc" | "docx" => ("Microsoft Word", "document"),
        "xls" | "xlsx" => ("Microsoft Excel", "workbook"),
        "ppt" | "pptx" => ("Microsoft PowerPoint", "presentation"),
        _ => return Err(format!("不支持的 Office 格式: {}", ext)),
    };
    try_applescript_print(app, obj, &path_str, false)
}

/// 执行一次 AppleScript 转换 Office → PDF，返回是否成功。
fn try_applescript_pdf(
    app_name: &str,
    obj_name: &str,
    save_line: &str,
    path_str: &str,
    pdf_path: &Path,
    pdf_posix: &str,
    use_posix: bool,
) -> Result<PathBuf, String> {
    let open_cmd = if use_posix {
        format!(r#"open POSIX file "{}""#, path_str)
    } else {
        format!(r#"open "{}""#, path_str)
    };
    let script = format!(
        r#"tell application "{}"
    with timeout of 60 seconds
        {}
        {}
        close active {} saving no
    end timeout
end tell"#,
        app_name, open_cmd, save_line, obj_name
    );

    log::info!(target: "office", "AppleScript 另存为 PDF: {} via {}", path_str, app_name);

    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("执行 osascript 失败: {}", e))?;

    if output.status.success() {
        if pdf_path.exists() {
            log::info!(target: "office", "AppleScript PDF 生成成功: {}", pdf_path.display());
            Ok(pdf_path.to_path_buf())
        } else {
            log::warn!(target: "office", "AppleScript 报告成功但 PDF 未生成: {}", pdf_posix);
            Err("AppleScript 报告成功但未找到 PDF 文件".into())
        }
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        log::warn!(target: "office", "AppleScript 另存为 PDF 失败 ({}): {}", app_name, err);
        Err(format!("AppleScript 另存为 PDF 失败 ({}): {}", app_name, err))
    }
}

/// 通过 AppleScript 将 Office 文档另存为 PDF（用于 build_pdf / Demo 模式）
/// 依次尝试：WPS → Microsoft Office，哪个先用哪个。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn office_to_pdf_via_applescript(input: &Path, output_dir: &Path) -> Result<PathBuf, String> {
    let abs = std::fs::canonicalize(input).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // 生成统一输出路径
    let now = chrono::Local::now();
    let pdf_name = format!("office_{}.pdf", now.format("%Y%m%d_%H%M%S_%3f"));
    let pdf_path = output_dir.join(&pdf_name);
    let pdf_posix = pdf_path.to_string_lossy().to_string();

    // WPS: macOS WPS 适配预留，当前版本 AppleScript 接口不完善
    // 后续可在此处插入 WPS 尝试逻辑

    // 再试 Microsoft Office
    let ms_config = match ext.as_str() {
        "doc" | "docx" => ("Microsoft Word", "document",
            format!(r#"save as active document file format format PDF file name (POSIX file "{}")"#, pdf_posix)),
        "xls" | "xlsx" => ("Microsoft Excel", "workbook",
            format!(r#"save active workbook in POSIX file "{}" as PDF file format"#, pdf_posix)),
        "ppt" | "pptx" => ("Microsoft PowerPoint", "presentation",
            format!(r#"save active presentation in (POSIX file "{}") as save as PDF"#, pdf_posix)),
        _ => return Err(format!("不支持的 Office 格式: {}", ext)),
    };
    let (app, obj, save_line) = ms_config;

    try_applescript_pdf(app, &obj, &save_line, &path_str, &pdf_path, &pdf_posix, false)
}
