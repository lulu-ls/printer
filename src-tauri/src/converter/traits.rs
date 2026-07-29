/// 转换器接口层
///
/// 所有文件 → PDF 的转换都通过 `Converter` trait 抽象，方便：
/// - 为同一格式注册多个候选转换器（择优选用）
/// - 后期替换底层实现（如 textutil → WKWebView → Chrome）而不改上层
///
/// 扩展名统一小写，不含点号，如 "html"、"pdf"。

use std::path::{Path, PathBuf};

// ── 转换结果 ────────────────────────────────────────

#[derive(Debug)]
pub struct ConvertOutput {
    /// 生成的 PDF 路径
    pub path: PathBuf,
    /// 实际处理的转换器名称（日志/调试用）
    #[allow(dead_code)]
    pub converter: &'static str,
}

// ── 转换器 trait ─────────────────────────────────────

pub trait Converter: Send + Sync {
    /// 转换器名称（日志/调试用）
    fn name(&self) -> &'static str;

    /// 是否支持此扩展名（扩展名已转小写，无点号）
    fn supports(&self, ext: &str) -> bool;

    /// 将文件转换为 PDF，输出到 output_dir。
    /// 扩展名已规范化小写，input 存在且非空。
    fn convert(&self, input: &Path, output_dir: &Path) -> Result<ConvertOutput, String>;

    /// 检查依赖是否就绪（如 LibreOffice 是否安装）。
    /// 返回 false 时该转换器会被跳过，不会触发 convert。
    fn available(&self) -> bool {
        true
    }
}

// ── 转换链（按注册顺序依次尝试） ─────────────────────

pub struct ConverterChain {
    converters: Vec<Box<dyn Converter>>,
}

impl ConverterChain {
    pub fn new() -> Self {
        Self {
            converters: Vec::new(),
        }
    }

    /// 注册一个转换器（优先级递减——先注册的先尝试）
    pub fn register(&mut self, converter: Box<dyn Converter>) {
        self.converters.push(converter);
    }

    /// 遍历所有转换器，找第一个支持此格式且可用的进行转换。
    /// 失败后自动尝试下一个，全部失败才返回错误。
    pub fn to_pdf(&self, input: &Path, output_dir: &Path) -> Result<ConvertOutput, String> {
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        for c in &self.converters {
            if c.supports(&ext) && c.available() {
                match c.convert(input, output_dir) {
                    Ok(out) => return Ok(out),
                    Err(e) => {
                        log::warn!(
                            target: "converter",
                            "{} 失败 ({}): {}，尝试下一个",
                            c.name(),
                            ext,
                            e
                        );
                    }
                }
            }
        }

        Err(format!("没有可用的转换器处理 .{} 文件", ext))
    }

    /// 返回处理此格式的首选转换器名称（前端提示用）
    #[allow(dead_code)]
    pub fn preferred_name(&self, ext: &str) -> &'static str {
        for c in &self.converters {
            if c.supports(ext) && c.available() {
                return c.name();
            }
        }
        "无可用转换器"
    }
}
