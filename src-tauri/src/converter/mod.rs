// 第一层：文件统一转换为 PDF
//
// 这是打印管线的入口。无论原始格式如何，都归一化为 PDF：
//   - PDF      直接
//   - 图片     JPEG 直接嵌入自生成 PDF；其它格式 macOS 经 sips 转 JPEG 嵌入，Windows/Linux 走 LibreOffice
//   - 文本     自生成 PDF（含非 Latin-1 时回退到系统原生文本打印）
//   - Office   LibreOffice headless 转换为 PDF
//
// 返回值：
//   Ok(path)   -> 待打印的 PDF 路径（可能就是原始文件，表示走原生打印）
//   Err(msg)   -> 真正失败（如缺少 LibreOffice）

pub mod image;
pub mod office;
pub mod text;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 统一转换入口。
pub fn to_pdf(input: &Path) -> Result<PathBuf, String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let tmp = temp_dir();

    match ext.as_str() {
        // PDF 直接使用
        "pdf" => Ok(input.to_path_buf()),

        // Office 文档 -> LibreOffice
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => {
            office::libreoffice_to_pdf(input, &tmp)
        }

        // 图片 -> 自生成 PDF
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" => {
            image::image_to_pdf(input, &tmp)
        }

        // 文本 -> 自生成 PDF（失败时回退原生打印）
        "txt" | "rtf" | "csv" => text::text_to_pdf(input, &tmp).or_else(|_| Ok(input.to_path_buf())),

        // 其它类型：尝试 LibreOffice，失败则直接用系统打印
        _ => office::libreoffice_to_pdf(input, &tmp).or_else(|_| Ok(input.to_path_buf())),
    }
}

/// 临时目录：系统临时目录下 /printer_assistant
pub fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("printer_assistant");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 生成唯一临时文件名（避免并发打印冲突）。
pub fn unique_name(prefix: &str, ext: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}_{}_{}.{}", prefix, ts, n, ext)
}
