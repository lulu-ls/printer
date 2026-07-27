// 第二层：PDF 渲染打印（平台分叉）
//
// 所有文件已统一为 PDF（或原生可打印文件），这里交给系统打印 API：
//   - macOS   : CUPS `lp`（系统原生、稳定、无 GUI）
//   - Windows : MuPDF `mutool draw -o printer:<name>`（推荐引擎，无第三方窗口）
//
// 统一入口：print_pdf(path, printer_name)

pub mod macos;
pub mod windows;

use std::path::Path;

pub fn print_pdf(path: &Path, printer: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos::print_via_lp(path, printer)
    }

    #[cfg(target_os = "windows")]
    {
        windows::print_via_mutool(path, printer)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (path, printer);
        Err("当前平台不支持打印".into())
    }
}
