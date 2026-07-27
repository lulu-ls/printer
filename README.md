# 打印机助手 (Printer Assistant)

一个轻量的跨平台桌面应用，把**图片 / Office 文档 / 文本**等文件直接发送到系统打印机打印。
无需打开文件对应的软件，选好文件和目标打印机，一键即可打印。

> 应用名：`打印机助手` ｜ 包标识：`com.printer.assistant` ｜ 版本：1.0.0

## 功能特性

- 选择本地文件（支持图片、Office、纯文本等），自动转换成 PDF
- 列出本机已安装的打印机，并可指定目标打印机
- 转换后的 PDF 通过系统打印服务直接输出到打印机
- 未检测到打印机时，自动打开「系统设置 → 打印机与扫描仪」引导添加
- 多语言界面，支持深色模式，窗口置顶

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面框架 | [Tauri 2](https://v2.tauri.app/)（Rust 后端 + 系统 WebView 前端） |
| 前端 | 原生 JavaScript + [Vite 6](https://vitejs.dev/)（无前端框架，轻量） |
| 后端 | Rust（Tauri 命令：`list_printers` / `print_file` / `build_pdf` / `open_url` 等） |
| 主要 Rust 依赖 | `tauri`、`tauri-plugin-dialog`、`serde`、`serde_json` |
| 系统工具（macOS） | `sips`（图片转 JPEG）、`lp` / `lpstat`（CUPS 打印与枚举） |
| 系统工具（打印 Office） | LibreOffice（`soffice`，文档转 PDF）、`mutool` / `pdfinfo`（PDF 元信息） |

工作原理：前端收集文件与打印机选择 → 调用 Tauri 命令 → 后端按文件类型转换为 PDF
（`image.rs` 用 sips + JPEG 内嵌、`office.rs` 用 LibreOffice、`text.rs` 用 PDF 库）→
最终用系统打印服务（`lp`）输出到打印机。

## 环境依赖

- **Node.js** 18+
- **Rust** 工具链（Stable，含 `cargo`）
- **Tauri CLI**（已作为 `devDependency` 通过 `npm` 脚本提供，无需单独安装）
- **macOS**：Xcode 命令行工具（`xcode-select --install`）
- **打印 Office 文档**：需安装 [LibreOffice](https://www.libreoffice.org/)（`soffice` 命令可用）
- Windows 额外依赖 `mutool`（用于 PDF 元信息读取）

## 快速开始

```bash
# 1. 安装依赖
npm install

# 2. 开发模式启动（同时拉起 Vite 前端 + Rust 后端，支持热重载）
npm run tauri dev

# 3. 构建可分发的应用（产物在 src-tauri/target/release/bundle/）
npm run tauri build

# 4. 运行：直接打开构建出的 .app（macOS）或安装包即可
```

> 仅启动前端（无原生能力，仅用于 UI 调试）：`npm run dev`，访问 http://localhost:1420
> 仅构建前端产物：`npm run build`（输出到 `dist/`）

## 调试

- **前端热重载**：`npm run tauri dev` 下修改 `src/` 自动刷新。
- **WebView 开发者工具**：开发模式下右键窗口或按快捷键打开浏览器调试面板，查看控制台与网络。
- **Rust 后端日志**：终端会实时输出 `cargo` 编译与运行日志；可用 `println!` / `eprintln!` 输出调试信息。
- **查看本机打印机**：终端执行 `lpstat -p`（列出）或 `lpstat -d`（默认打印机）。
- **查看转换后的 PDF**：`build_pdf` 命令可单独生成 PDF 到临时目录，便于核对渲染效果。

## 项目结构

```
printer/
├── index.html              # 前端入口
├── vite.config.mjs         # Vite 开发/构建配置
├── src/                    # 前端（原生 JS）
│   ├── main.js             # 主逻辑：选文件、打印机、打印流程
│   ├── i18n.js             # 多语言文案
│   └── styles.css          # 样式（含深色模式）
├── src-tauri/              # Rust 后端（Tauri）
│   ├── tauri.conf.json     # 应用配置（窗口、包名、构建）
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/
│       ├── lib.rs          # 命令注册入口
│       ├── pdf.rs          # PDF 生成（图片嵌入等）
│       ├── printer/        # 平台相关打印（macos/windows）
│       └── converter/      # 文件转 PDF（image/office/text）
└── doc/                    # 设计文档
```

## 常见问题

- **提示“未检测到打印机”**：本机 CUPS 中没有已添加的打印机。点击「开始打印」会自动打开系统打印设置，添加后重启应用即可。
- **Office 文件打印失败**：确认已安装 LibreOffice 且 `soffice` 在 `PATH` 中。
- **图片转出的 PDF 无法打开**：已由 `pdf.rs` 中 JPEG 嵌入对象编号修正，请使用包含该修复的版本。
