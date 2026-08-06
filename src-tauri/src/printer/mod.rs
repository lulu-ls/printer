// 第二层：PDF 渲染打印（平台分叉）
//
// 所有文件已统一为 PDF（或原生可打印文件），这里交给系统打印 API：
//   - macOS   : CUPS `lp`（系统原生、稳定、无 GUI）
//   - Windows : MuPDF `mutool draw -o printer:<name>`（推荐引擎，无第三方窗口）
//
// 统一入口：print_pdf(path, printer_name, settings)

pub mod macos;
pub mod windows;

use std::path::Path;

/// 打印设置（由前端传入）
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintSettings {
    /// 份数，默认 1
    pub copies: u32,
    /// 彩色（true=彩色 / false=黑白），默认 true
    pub color: bool,
    /// 双面（true=双面长边装订），默认 false
    pub duplex: bool,
    /// 横向（true=横向 / false=纵向），默认 false
    pub landscape: bool,
}

impl PrintSettings {
    /// 后端从空值解析时的默认设置
    pub fn from_optional(o: Option<&PrintSettings>) -> Self {
        o.cloned().unwrap_or_default()
    }
}

/// 默认打印设置：1 份、彩色、单面、纵向
impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            copies: 1,
            color: true,
            duplex: false,
            landscape: false,
        }
    }
}

pub fn print_pdf(path: &Path, printer: &str, settings: &PrintSettings) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos::print_via_lp(path, printer, settings)
    }

    #[cfg(target_os = "windows")]
    {
        windows::print_via_mutool(path, printer, settings)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (path, printer, settings);
        Err("当前平台不支持打印".into())
    }
}
