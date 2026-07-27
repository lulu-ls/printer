// 图片 -> PDF
//
// JPEG（jpg/jpeg）是压缩格式，可直接以 DCTDecode 嵌入手写 PDF（零质量损失、最快、
// 无需任何外部依赖），全平台适用。
// 其它图片格式：macOS 用系统 sips 转 JPEG 后再嵌入；Windows/Linux 无 sips，退回
// LibreOffice 转 PDF。

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::process::Command;

use crate::converter::unique_name;
use crate::pdf;

pub fn image_to_pdf(input: &Path, tmp: &Path) -> Result<PathBuf, String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // JPEG 直接嵌入，全平台通用，无需 LibreOffice
    if ext == "jpg" || ext == "jpeg" {
        return jpeg_to_pdf_file(input, tmp);
    }

    #[cfg(target_os = "macos")]
    {
        let jpeg = jpeg_bytes_for(input, tmp)?;
        jpeg_to_pdf_file_with(input, &jpeg, tmp)
    }

    #[cfg(not(target_os = "macos"))]
    {
        use crate::converter::{office, temp_dir};
        let _ = tmp;
        office::libreoffice_to_pdf(input, &temp_dir())
    }
}

/// 直接把 JPEG 字节封装为 PDF 文件（零质量损失）。
fn jpeg_to_pdf_file(input: &Path, tmp: &Path) -> Result<PathBuf, String> {
    let jpeg = std::fs::read(input).map_err(|e| e.to_string())?;
    jpeg_to_pdf_file_with(input, &jpeg, tmp)
}

/// 用给定的 JPEG 字节生成 PDF 文件并写入临时目录。
fn jpeg_to_pdf_file_with(input: &Path, jpeg: &[u8], tmp: &Path) -> Result<PathBuf, String> {
    let pdf = pdf::jpeg_to_pdf(jpeg).map_err(|e| format!("生成 PDF 失败：{}", e))?;
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".into());
    let out = tmp.join(unique_name(&stem, "pdf"));
    std::fs::write(&out, pdf).map_err(|e| e.to_string())?;
    Ok(out)
}

/// 取得可用于嵌入的 JPEG 字节（macOS：sips 转 JPEG；JPEG 直接读）。
#[cfg(target_os = "macos")]
fn jpeg_bytes_for(input: &Path, tmp: &Path) -> Result<Vec<u8>, String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if ext == "jpg" || ext == "jpeg" {
        return std::fs::read(input).map_err(|e| e.to_string());
    }

    let out_jpg = tmp.join(unique_name("img", "jpg"));
    let status = Command::new("sips")
        .args(["-s", "format", "jpeg", "--out"])
        .arg(&out_jpg)
        .arg(input)
        .status()
        .map_err(|e| format!("sips 调用失败：{}", e))?;
    if !status.success() {
        return Err("sips 转换图片失败".into());
    }
    std::fs::read(&out_jpg).map_err(|e| e.to_string())
}
