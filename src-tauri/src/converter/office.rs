// Office / 通用文档 -> PDF
//
// 使用 LibreOffice headless 把 docx/xlsx/pptx 等转换为 PDF。
// 用户完全无感知（后台进程，不打开任何窗口）。
//
// 注意：通过 DMG / 官方安装包安装的 LibreOffice 默认不在 PATH 中：
//   - macOS:   /Applications/LibreOffice.app/Contents/MacOS/soffice
//   - Windows: C:\Program Files\LibreOffice\program\soffice.exe
// 因此这里除了尝试 `libreoffice` / `soffice` 命令，还会探测常见安装路径。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 查找可用的 LibreOffice 可执行文件路径。
/// 依次尝试：PATH 中的命令名 + 各平台常见安装路径。
/// 返回第一个可用的命令（或绝对路径）。
fn find_libreoffice() -> Option<String> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "libreoffice",
            "soffice",
            "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "libreoffice",
            "soffice",
            "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
            "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
        ]
    } else {
        &["libreoffice", "soffice"]
    };

    for b in candidates {
        // 绝对路径：直接检查文件是否存在（更快、更可靠，且不启动 LO 进程）
        // PATH 中的命令：尝试执行 `--version` 确认可用
        let available = if b.contains('\\') || b.starts_with('/') {
            Path::new(b).is_file()
        } else {
            Command::new(b).arg("--version").status().is_ok()
        };
        if available {
            return Some(b.to_string());
        }
    }
    None
}

pub fn libreoffice_to_pdf(input: &Path, tmp: &Path) -> Result<PathBuf, String> {
    let bin = find_libreoffice().ok_or_else(|| {
        "未检测到 LibreOffice（libreoffice / soffice）。请安装 LibreOffice 以打印 Office 文档。"
            .to_string()
    })?;

    let status = Command::new(&bin)
        .args([
            "--headless",
            "--norestore",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(tmp)
        .arg(input)
        .status();

    match status {
        Ok(s) if s.success() => {
            // 输出文件名 = 原 stem + .pdf（放在 tmp）
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "document".into());
            let out = tmp.join(format!("{}.pdf", stem));
            if out.exists() {
                return Ok(out);
            }
            // LibreOffice 偶尔放到输入同目录，找一下
            let alt = input.with_extension("pdf");
            if alt.exists() {
                let moved = tmp.join(format!("{}.pdf", stem));
                let _ = std::fs::rename(&alt, &moved);
                return Ok(moved);
            }
            Err("LibreOffice 未生成 PDF 输出".into())
        }
        _ => Err(format!("LibreOffice 转换失败：{:?}", status)),
    }
}

/// 检查本机是否可用 LibreOffice（供前端提示）。
pub fn libreoffice_available() -> bool {
    find_libreoffice().is_some()
}

/// 判断某文件是否需要 LibreOffice 才能打印。
/// - 所有平台：Office 文档（doc/docx/xls/xlsx/ppt/pptx）
/// - 非 macOS：png/gif/bmp/tif/webp 等图片（JPEG 已可直接嵌入，无需 LibreOffice）、
///   文本（txt/rtf/csv）也走 LibreOffice（macOS 有 sips / 原生文本打印）
pub fn requires_libreoffice(input: &Path) -> bool {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let office = matches!(
        ext.as_str(),
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx"
    );

    // Markdown 所有平台都需要 LibreOffice（无原生工具）
    let md = ext == "md";

    #[cfg(target_os = "macos")]
    {
        // macOS：HTML 可用 textutil，无需 LO；md 需要
        office || md
    }

    #[cfg(not(target_os = "macos"))]
    {
        office
            || md
            || matches!(
                ext.as_str(),
                "png"
                    | "gif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "webp"
                    | "txt"
                    | "rtf"
                    | "csv"
                    | "html"
                    | "htm"
            )
    }
}
