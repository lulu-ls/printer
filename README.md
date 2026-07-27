# Printer Assistant

**跨平台桌面打印工具** — 拖拽文件即可打印，无需打开对应的应用。

把 PDF、Office 文档、图片、文本文件直接发送到系统打印机，一键完成。基于 Tauri 2 构建，Rust 后端 + 原生 WebView 前端，轻量无依赖。

> 包标识：`com.printer.assistant` · 版本：1.0.0

---

## 截图

![screenshot](doc/screenshot.png)

## 核心特性

- **拖拽即打**：拖入文件或点击选择，支持 PDF、Office（doc/docx/xls/xlsx/ppt/pptx）、图片（png/jpg/gif/bmp/webp/tiff）、纯文本
- **智能转换管线**：文件自动转为 PDF 后送打印机，全程无需打开第三方软件
- **自动回退**：macOS 优先用 Microsoft Office（AppleScript）静默打印 Office 文档，未安装时自动回退 LibreOffice
- **打印机管理**：自动检测本机打印机、在线状态、默认打印机，弹窗切换
- **跨平台**：macOS（CUPS `lp`）、Windows（MuPDF `mutool`）
- **多语言**：中文 / English，原生菜单切换

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面框架 | [Tauri 2](https://v2.tauri.app/)（Rust 后端 + 系统 WebView） |
| 前端 | 原生 JavaScript + [Vite 6](https://vitejs.dev/) |
| 后端 | Rust（打印管线、文件转换、系统调用） |
| 打印（macOS） | CUPS `lp` / `lpstat` + AppleScript（Microsoft Office） |
| 打印（Windows） | MuPDF `mutool draw -o printer:<name>` |
| Office 转 PDF | LibreOffice `soffice --headless --convert-to pdf` |
| 图片处理（macOS） | 系统 `sips` 转 JPEG |

## 快速开始

```bash
# 1. 安装依赖
npm install

# 2. 开发模式（热重载）
npm run tauri dev

# 3. 构建分发 输出目录为 src-tauri/target/release
npm run tauri build
```

依赖：Node.js 18+、Rust 工具链、macOS 需 Xcode CLI（`xcode-select --install`）。


## 环境依赖

- **macOS**：系统自带 CUPS。Office 文档建议安装 Microsoft Office 或 [LibreOffice](https://www.libreoffice.org/)
- **Windows**：需 [MuPDF](https://mupdf.com/)（`mutool` 在 PATH 中）。Office 文档建议安装 WPS / Office / libreoffice

## License

MIT
