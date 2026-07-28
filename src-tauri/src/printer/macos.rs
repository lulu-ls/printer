// macOS 打印：CUPS `lp` + AppleScript 打印 Office 文档
//
// 系统原生打印管线，稳定且不会弹出任何第三方窗口（不使用 Preview / osascript）。
// Office 文档（doc/docx/xls/xlsx/ppt/pptx）优先通过 AppleScript 调用
// Microsoft Office 直接打印，避免依赖 LibreOffice。

use std::path::Path;
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

/// 检测 macOS 上 Microsoft Office 是否可用（Word / Excel / PowerPoint）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_office_available() -> bool {
    // 检测 Word 即可代表 Office 套件是否安装（通常三者同时安装或都不装）
    // 用 `mdfind` 查找 .app 包比 osascript 调用更快、不启动任何进程
    Command::new("mdfind")
        .args(["kMDItemKind == \"Application\" && kMDItemFSName == \"Microsoft Word.app\""])
        .output()
        .ok()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.trim().contains("Microsoft Word")
        })
        .unwrap_or(false)
}

/// 通过 AppleScript 静默打印 Office 文档（不弹出任何窗口）。
/// 返回 Ok 表示已成功提交打印，Err 表示失败（将回退 LibreOffice）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn print_office_via_applescript(input: &Path) -> Result<String, String> {
    let abs = std::fs::canonicalize(input).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // 根据扩展名选择对应的 Office 应用
    let app = match ext.as_str() {
        "doc" | "docx" => "Microsoft Word",
        "xls" | "xlsx" => "Microsoft Excel",
        "ppt" | "pptx" => "Microsoft PowerPoint",
        _ => return Err(format!("不支持的 Office 格式: {}", ext)),
    };

    // AppleScript：打开文档 → 静默打印 → 关闭不保存
    // 使用「with timeout」避免长时间挂起；print out 使用默认打印机（静默）
    let script = format!(
        r#"tell application "{}"
    with timeout of 30 seconds
        set doc to open "{}"
        print out doc
        close doc saving no
    end timeout
end tell"#,
        app, path_str
    );

    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("执行 osascript 失败: {}", e))?;

    if output.status.success() {
        Ok(format!("{} 已通过 {} 打印", path_str, app))
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("AppleScript 打印失败 ({}): {}", app, err))
    }
}
