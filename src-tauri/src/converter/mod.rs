// 第一层：文件统一转换为 PDF
//
// 这是打印管线的入口。无论原始格式如何，都归一化为 PDF。
// 通过 Converter trait 抽象，各格式可注册多个候选转换器择优选用。
//
// 返回值：
//   Ok(path)   -> 待打印的 PDF 路径（可能就是原始文件，表示走原生打印）
//   Err(msg)   -> 真正失败

pub mod image;
pub mod office;
pub mod text;
pub mod traits;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use traits::{Converter, ConverterChain};

static SEQ: AtomicU64 = AtomicU64::new(0);

// ── 全局转换链（惰性初始化） ─────────────────────────

fn chain() -> &'static ConverterChain {
    use std::sync::OnceLock;
    static CHAIN: OnceLock<ConverterChain> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let mut c = ConverterChain::new();

        // PDF —— 直接复制
        c.register(Box::new(PassthroughPdf));

        // 图片 —— JPEG 直接嵌入 PDF；其他格式 macOS 经 sips 转 JPEG
        c.register(Box::new(ImageConverter));

        // HTML —— macOS 用 WKWebView 完美渲染；其他走 LibreOffice
        c.register(Box::new(HtmlWebViewConverter));

        // Office 文档 —— LibreOffice
        c.register(Box::new(OfficeConverter));

        // 纯文本 —— 自生成 PDF（失败回退原生打印）
        c.register(Box::new(TextConverter));

        // Markdown —— 暂走 LibreOffice
        c.register(Box::new(LibreOfficeFallback {
            exts: &["md"],
            name: "markdown",
        }));

        // 万能兜底 —— 尝试 LibreOffice，失败则丢回系统打印
        c.register(Box::new(LibreOfficeFallback {
            exts: &[],
            name: "universal",
        }));

        c
    })
}

/// 统一转换入口。
pub fn to_pdf(input: &Path) -> Result<PathBuf, String> {
    let tmp = temp_dir();
    let result = chain().to_pdf(input, &tmp)?;
    Ok(result.path)
}

// ── 内置转换器实现 ───────────────────────────────────

/// PDF 直通：不做任何转换
struct PassthroughPdf;
impl Converter for PassthroughPdf {
    fn name(&self) -> &'static str {
        "passthrough-pdf"
    }
    fn supports(&self, ext: &str) -> bool {
        ext == "pdf"
    }
    fn convert(&self, input: &Path, _output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        Ok(traits::ConvertOutput {
            path: input.to_path_buf(),
            converter: self.name(),
        })
    }
}

/// 图片转换器
struct ImageConverter;
impl Converter for ImageConverter {
    fn name(&self) -> &'static str {
        "image"
    }
    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp")
    }
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        image::image_to_pdf(input, output_dir).map(|path| traits::ConvertOutput {
            path,
            converter: self.name(),
        })
    }
}

/// HTML WebView 转换器（macOS 用 WKWebView，其他平台用 LibreOffice）
struct HtmlWebViewConverter;
impl Converter for HtmlWebViewConverter {
    fn name(&self) -> &'static str {
        "html-webview"
    }
    fn supports(&self, ext: &str) -> bool {
        ext == "html" || ext == "htm"
    }
    fn available(&self) -> bool {
        #[cfg(target_os = "macos")]
        return true;
        #[cfg(not(target_os = "macos"))]
        return office::libreoffice_available();
    }
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        crate::html_webview::html_to_pdf(input, output_dir).map(|path| traits::ConvertOutput {
            path,
            converter: self.name(),
        })
    }
}

/// Office 转换器
struct OfficeConverter;
impl Converter for OfficeConverter {
    fn name(&self) -> &'static str {
        "office"
    }
    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx")
    }
    fn available(&self) -> bool {
        office::libreoffice_available()
    }
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        office::libreoffice_to_pdf(input, output_dir).map(|path| traits::ConvertOutput {
            path,
            converter: self.name(),
        })
    }
}

/// 纯文本转换器
struct TextConverter;
impl Converter for TextConverter {
    fn name(&self) -> &'static str {
        "text"
    }
    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "txt" | "rtf" | "csv")
    }
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        text::text_to_pdf(input, output_dir)
            .or_else(|_| Ok(input.to_path_buf()))
            .map(|path| traits::ConvertOutput {
                path,
                converter: self.name(),
            })
    }
}

/// LibreOffice 兜底转换器（用于 md 等 LO 可打开的格式）
struct LibreOfficeFallback {
    exts: &'static [&'static str],
    name: &'static str,
}
impl Converter for LibreOfficeFallback {
    fn name(&self) -> &'static str {
        self.name
    }
    fn supports(&self, ext: &str) -> bool {
        if self.exts.is_empty() {
            true // 万能兜底
        } else {
            self.exts.contains(&ext)
        }
    }
    fn available(&self) -> bool {
        office::libreoffice_available() || self.exts.is_empty()
    }
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<traits::ConvertOutput, String> {
        office::libreoffice_to_pdf(input, output_dir)
            .or_else(|_| Ok(input.to_path_buf()))
            .map(|path| traits::ConvertOutput {
                path,
                converter: self.name(),
            })
    }
}

// ── 工具函数 ─────────────────────────────────────────

pub fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("printer_assistant");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn unique_name(prefix: &str, ext: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}_{}_{}.{}", prefix, ts, n, ext)
}
