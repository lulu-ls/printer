// Windows 打印：MuPDF `mutool`
//
// 使用文档推荐的 MuPDF 引擎，直接把 PDF 投递到 Windows 打印后台，
// 不调用 start / Adobe Reader / Edge 等任何第三方窗口。
//
// 前置：系统需安装 MuPDF 并把 `mutool` 加入 PATH（或随应用分发）。
//   mutool draw -o "printer:<打印机名>" input.pdf

use std::path::Path;
use std::process::Command;

#[allow(dead_code)]
pub fn print_via_mutool(path: &Path, printer: &str) -> Result<String, String> {
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let path_str = abs.to_string_lossy().to_string();

    // 输出目标：printer:<名称>，名称为空则使用默认打印机
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
