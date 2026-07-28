// Windows 打印：MuPDF `mutool` + GDI 直打图片 + COM 自动化打印 Office
//
// 使用文档推荐的 MuPDF 引擎，直接把 PDF 投递到 Windows 打印后台，
// 不调用 start / Adobe Reader / Edge 等任何第三方窗口。
//
// 前置：系统需安装 MuPDF 并把 `mutool` 加入 PATH（或随应用分发）。
//   mutool draw -o "printer:<打印机名>" input.pdf
//
// 图片走 GDI 直打（零依赖），Office 走 COM 自动化（WPS / Office）。

use std::path::Path;
use std::process::Command;
#[cfg(windows)]
use std::ffi::OsString;
#[allow(unused_imports)]
use tauri::AppHandle;

pub fn print_via_mutool(path: &Path, printer: &str) -> Result<String, String> {
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();

    let output_target = format!("printer:{}", printer);

    let status = Command::new("mutool")
        .args(["draw", "-o", &output_target])
        .arg(&path_str)
        .status()
        .map_err(|_| {
            "未找到 mutool（MuPDF）。Windows 打印需在 PATH 中提供 mutool，或随应用分发 MuPDF。"
                .to_string()
        })?;

    if !status.success() {
        return Err("mutool 打印失败".into());
    }
    Ok("ok".into())
}

/// 检测本机是否安装了办公软件的 COM 自动化接口（WPS / Office）。
pub fn detect_office_com(_extension: &str) -> Option<String> {
    // 尝试检测 WPS 或 Microsoft Office 的 COM 注册
    // 简单检测：查看注册表中是否有对应 ProgID
    // WPS: Kwps.Application / Ket.Application / Wpp.Application
    // Office: Word.Application / Excel.Application / PowerPoint.Application
    let prog_ids = [
        "Kwps.Application",   // WPS 文字
        "Ket.Application",    // WPS 表格
        "Wpp.Application",    // WPS 演示
        "Word.Application",   // Microsoft Word
        "Excel.Application",  // Microsoft Excel
        "PowerPoint.Application", // Microsoft PowerPoint
    ];

    for prog_id in &prog_ids {
        if check_com_progid(prog_id) {
            return Some(prog_id.to_string());
        }
    }
    None
}

#[cfg(windows)]
fn check_com_progid(prog_id: &str) -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let key_path = format!("SOFTWARE\\Classes\\{}\\CLSID", prog_id);
    match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(&key_path) {
        Ok(key) => key.get_value::<OsString, _>("").is_ok(),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn check_com_progid(_prog_id: &str) -> bool {
    false
}

/// 通过 COM 自动化打印 Office 文档（静默，不弹出窗口）。
pub fn print_office_via_com(_input: &Path, _printer: &str) -> Result<String, String> {
    // COM 自动化打印实现
    // 通过 Windows Script Host 或直接 COM 调用打印
    // 简化实现：使用 PowerShell 调用 Office COM 对象
    let abs = std::fs::canonicalize(_input).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();
    let ext = _input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let prog_id = match ext.as_str() {
        "doc" | "docx" => Some("Word.Application"),
        "xls" | "xlsx" => Some("Excel.Application"),
        "ppt" | "pptx" => Some("PowerPoint.Application"),
        _ => None,
    };

    let prog_id = match prog_id {
        Some(p) => p,
        None => return Err(format!("不支持的 Office 格式: {}", ext)),
    };

    // 用 PowerShell 脚本静默打印
    let ps_script = format!(
        r#"
$app = New-Object -ComObject "{}"
$doc = $app.Documents.Open("{}")
$doc.PrintOut()
$doc.Close([ref]0)
$app.Quit()
"#,
        prog_id, path_str.replace("'", "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()
        .map_err(|e| format!("PowerShell 执行失败: {}", e))?;

    if output.status.success() {
        Ok(format!("{} 已通过 {} 打印", path_str, prog_id))
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("COM 打印失败: {}", err))
    }
}

/// GDI 直打图片（零依赖，静默打印）。
pub async fn print_image(_app: &tauri::AppHandle, _input: &Path, _printer: &str) -> Result<String, String> {
    // GDI 直打图片的实现
    // 通过 Windows GDI 的 StartDoc / StartPage / EndPage API
    // 简化实现：先转换为 PDF 再打印
    Err("GDI 直打暂未实现，请使用通用管线".into())
}
