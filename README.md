# 拖进去直接打印，免去打开各类软件的烦恼

每次从微信打印孩子作业，打印公司文件，先从聊天工具保存到桌面，然后图片、excel、doc等不同的文件要先打开不同的预览软件，然后再点击打印，文件多了就很烦，所以做了这个软件

如果帮助到你，欢迎点个 star 
---

## 演示
<img width="400" height="225" alt="demo-ezgif com-video-to-gif-converter" src="https://github.com/user-attachments/assets/3d6d7f43-773c-4972-b2f3-cf44c6c7cee7" />


## 截图
<img width="558" height="368" alt="image" src="https://github.com/user-attachments/assets/27c0a4ab-7a20-4f7e-84b6-9c7d48a27002" />

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

## 环境依赖

- **macOS**：系统自带 CUPS。Office 文档建议安装 Microsoft Office 或 [LibreOffice](https://www.libreoffice.org/)
- **Windows**：需 [MuPDF](https://mupdf.com/)（`mutool` 在 PATH 中）。Office 文档建议安装 WPS / Office / libreoffice

## 交流

	-- 如果你有什么问题欢迎和我联系（raxzib@gmail.com），或者提交 issue
    -- 如果对你有帮助，欢迎 star 支持

## License

MIT
