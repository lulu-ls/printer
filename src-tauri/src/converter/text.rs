// 文本 -> PDF
//
// 用手写 PDF 生成器排版文本。若文本含非 Latin-1 字符（如中文），
// PDF 生成会失败，此时返回 Err，由上层回退到系统原生文本打印（CUPS `lp`）。

use std::path::{Path, PathBuf};

use crate::converter::unique_name;
use crate::pdf;

pub fn text_to_pdf(input: &Path, tmp: &Path) -> Result<PathBuf, String> {
    let content = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
    let bytes = pdf::text_to_pdf(&content)?; // 失败 -> Err -> 上层回退原生打印

    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "text".into());
    let out = tmp.join(unique_name(&stem, "pdf"));
    std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    Ok(out)
}
