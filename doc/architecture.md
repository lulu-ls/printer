# 打印机助手 — 设计架构文档

> 版本：1.0.0 · 更新日期：2026-07-27

---

## 1. 概述

打印机助手（Printer Assistant）是一个跨平台桌面应用，提供"拖拽文件 → 自动转换 → 系统打印"的一站式打印体验。用户无需打开 Word、Excel、Photoshop 等应用，即可将各类文档直接发送到打印机。

**核心设计原则：**

- **零窗口**：全程不弹出任何第三方应用窗口（LibreOffice、Microsoft Office、Preview 等均在后台静默运行）
- **自动回退**：优先使用本机已装软件（Microsoft Office / WPS）打印，失败时自动回退 LibreOffice
- **轻量无框架**：前端使用原生 JavaScript，无 React/Vue 等框架依赖

---

## 2. 系统架构

```
┌─────────────────────────────────────────────────────────┐
│                    用户界面 (WebView)                    │
│  index.html · main.js · styles.css · i18n.js            │
│  ┌──────────┐  ┌────────────┐  ┌────────────────────┐   │
│  │ 拖拽/选择 │→│ 文件列表   │→│ 打印机选择 + 打印        │   │
│  │ 文件      │  │ 管理       │  │ 按钮                 │   │
│  └──────────┘  └────────────┘  └────────┬───────────┘   │
│                                         │ invoke()      │
└─────────────────────────────────────────┼───────────────┘
                                          │ Tauri IPC
┌─────────────────────────────────────────┼───────────────┐
│                Rust 后端                 │               │
│  ┌──────────────────────────────────────┘               │
│  │  print_file(path, printer_name)                      │
│  │                                                      │
│  │  ├─ [macOS] AppleScript 打印 Office 文档              │
│  │  │   └─ 失败回退 ↓                                    │
│  │  ├─ [Windows] COM 自动化打印 Office 文档               │
│  │  │   └─ 失败回退 ↓                                    │
│  │  │                                                   │
│  │  ├─ converter::to_pdf(input)  ← 统一转 PDF            │
│  │  │   ├─ PDF          → 直接使用                      │
│  │  │   ├─ Office 文档  → LibreOffice --headless        │
│  │  │   ├─ 图片        → sips + 自生成 PDF               │
│  │  │   └─ 文本        → 自生成 PDF                      │
│  │  │                                                  │
│  │  └─ printer::print_pdf(pdf, printer_name)           │
│  │      ├─ macOS:  CUPS `lp`                           │
│  │      └─ Windows: MuPDF `mutool draw`                │
│  │                                                     │
│  └─────────────────────────────────────────────────────┘
```

---

## 3. 模块详解

### 3.1 前端模块

#### 3.1.1 入口与状态管理

**文件**：`index.html` + `src/main.js`

前端采用单页应用（SPA）模式，状态集中管理在一个 `state` 对象中：

```javascript
const state = {
  files: [],           // { id, path, name, size, ext, status, error }
  printers: [],        // 打印机名称列表
  printerName: '',     // 当前选中打印机
  printerOnline: null, // 当前打印机在线状态 (true/false/null)
  printerStatuses: {}, // 全部打印机在线状态 { name: bool }
  noPrinter: false,    // 是否无可用打印机
  printing: false,     // 是否正在打印
  lang: 'zh',          // 当前语言
};
```

**核心 UI 状态切换**：

```
空状态 (empty-state)  ↔  文件列表页 (file-page)
    ↑                         ↑
  showEmpty()            showFileList() / renderFiles()
```

#### 3.1.2 文件管理流程

```
用户拖入/选择文件
  │
  ├─ 类型过滤 (supported.includes(ext))
  │   └─ 不支持的格式 → toast 提示
  │
  ├─ addFile(path) → invoke('get_file_info')
  │   ├─ 去重（按路径）
  │   ├─ 生成唯一 id
  │   └─ 插入 state.files 数组
  │
  └─ renderFiles()
      ├─ 倒序遍历数组（最新文件在最上方）
      ├─ 增量渲染：新卡片播入场动画，已有卡片只更新状态
      └─ 被删除卡片播退场动画后移除
```

#### 3.1.3 打印流程

```
startPrint()
  │
  ├─ 无打印机 → 打开系统打印机设置，返回
  │
  ├─ 检测 office_automation_available()
  │   └─ macOS: 检测 Microsoft Office 是否安装
  │   └─ Windows: 检测 COM 自动化可用性
  │
  ├─ 遍历文件调用 needs_libreoffice()
  │   ├─ 需要 LO 且未装 Office → 弹出 LibreOffice 下载提示
  │   │   ├─ 下载：打开浏览器下载页
  │   │   ├─ 跳过：过滤掉需要 LO 的文件
  │   │   └─ 取消：终止打印
  │   └─ 不需要 → 加入待打印列表
  │
  └─ 逐个调用 invoke('print_file', { path, printerName })
      ├─ 成功：粒子爆散动画 + 卡片退场
      └─ 失败：标记 fail，保留在列表
```

#### 3.1.4 打印机管理

```
打印机卡片 (printer-status)
  ├─ 无打印机 → 显示"暂未检测到打印机"，点击打开系统设置
  └─ 有打印机 → 显示当前打印机名 + 在线状态圆点
       ├─ 点击弹出打印机选择列表
       └─ 选择后切换打印机

状态轮询 (每 3 秒)
  └─ refreshStatuses() → invoke('printers_status')
       └─ 更新 state.printerStatuses + UI 圆点
```

#### 3.1.5 顶部添加区域折叠

当文件列表内容溢出时，滚动列表会使顶部 dropzone 折叠为窄条，吸顶固定。采用迟滞（hysteresis）机制避免临界抖动：

- 展开→折叠：`scrollTop > 40px`
- 折叠→展开：`scrollTop <= 5px`（回到顶部附近）
- 使用 `requestAnimationFrame` 节流 scroll 事件

### 3.2 后端模块

#### 3.2.1 命令注册

**文件**：`src-tauri/src/lib.rs`

注册 14 个 Tauri 命令：

| 命令 | 功能 | 平台差异 |
|------|------|---------|
| `list_printers` | 枚举本机打印机 | macOS: `lpstat -p`；Windows: `EnumPrintersW` |
| `get_default_printer` | 获取默认打印机 | macOS: `lpstat -d`；Windows: `GetDefaultPrinterW` |
| `get_file_info` | 获取文件元信息 | 无差异 |
| `print_file` | 核心打印管线 | 见下文 |
| `libreoffice_available` | 检测 LibreOffice | 各平台检测不同路径 |
| `office_automation_available` | 检测办公软件自动化 | macOS: AppleScript；Windows: COM |
| `needs_libreoffice` | 文件是否需要 LO | macOS: 仅 Office 文档；其他平台: 含图片/文本 |
| `build_pdf` | 仅转 PDF 不打印 | 无差异 |
| `open_url` | 系统打开 URL | 各平台不同命令 |
| `platform` | 返回当前平台 | — |
| `printer_online` | 单台打印机在线检测 | 见上 |
| `printers_status` | 批量打印机状态 | 见上 |
| `get_language` | 读取语言偏好 | 持久化到文件 |
| `log_message` | 前端写日志 | — |

#### 3.2.2 打印管线 (`print_file`)

```
print_file(path, printer_name)
  │
  ├─ [Windows 优先] ──────────────────────
  │   ├─ 图片 → GDI 直打 (print_image)
  │   └─ Office → COM 自动化 (print_office_via_com)
  │
  ├─ [macOS 优先] ────────────────────────
  │   └─ Office → AppleScript (print_office_via_applescript)
  │       ├─ 调用对应 Microsoft Office 应用
  │       ├─ 静默打印（不弹出窗口）
  │       └─ 30 秒超时保护
  │
  ├─ 通用管线 ────────────────────────────
  │   └─ converter::to_pdf(input)
  │       ├─ PDF → 直接返回
  │       ├─ Office → LibreOffice --headless
  │       ├─ 图片 → sips 转 JPEG + 自生成 PDF
  │       ├─ 文本 → 自生成 PDF
  │       └─ 其他 → 尝试 LibreOffice
  │
  └─ printer::print_pdf(pdf, printer_name)
      ├─ macOS:  CUPS `lp -d <printer> <file>`
      └─ Windows: MuPDF `mutool draw -o printer:<name> <file>`
```

**自动回退机制**：每个平台优先路径失败后均会回退到通用管线，确保高可用性。

#### 3.2.3 文件转换模块

**目录**：`src-tauri/src/converter/`

| 文件 | 类 | 功能 | 依赖 |
|------|---|------|------|
| `mod.rs` | — | 统一入口 `to_pdf(input)` | 分派到子模块 |
| `office.rs` | LibreOffice | Office 文档 → PDF | `soffice --headless` |
| `image.rs` | 图片转 PDF | 图片 → JPEG → PDF | macOS: `sips`；其他: LibreOffice |
| `text.rs` | 文本转 PDF | 纯文本 → PDF | 自生成（Latin-1）；含中文回退 |

**LibreOffice 检测路径**：

| 平台 | 检测顺序 |
|------|---------|
| macOS | `libreoffice` → `soffice` → `/Applications/LibreOffice.app/Contents/MacOS/soffice` |
| Windows | `libreoffice` → `soffice` → `C:\Program Files\LibreOffice\program\soffice.exe` → `C:\Program Files (x86)\...` |
| Linux | `libreoffice` → `soffice` |

**文件类型支持列表**：

| 类别 | 扩展名 | 转换方式 |
|------|--------|---------|
| PDF | `pdf` | 直接使用 |
| Office | `doc` `docx` `xls` `xlsx` `ppt` `pptx` | LibreOffice |
| 图片 | `jpg` `jpeg` `png` `gif` `bmp` `tif` `tiff` `webp` | macOS: sips + 自生成 PDF；其他: LibreOffice |
| 文本 | `txt` `rtf` `csv` | 自生成 PDF（Latin-1）；含中文回退原生打印 |

#### 3.2.4 打印模块

**目录**：`src-tauri/src/printer/`

| 文件 | 平台 | 打印方式 | 特点 |
|------|------|---------|------|
| `mod.rs` | — | `print_pdf()` 分派 | 编译期平台分叉 |
| `macos.rs` | macOS | CUPS `lp` + AppleScript | 原生 CUPS，零额外依赖 |
| `windows.rs` | Windows | MuPDF `mutool draw` | 需安装 MuPDF |

**macOS AppleScript 静默打印**：

```
检测 Office 应用 → 构造 AppleScript → osascript 执行
  ├─ .doc/.docx  → Microsoft Word
  ├─ .xls/.xlsx  → Microsoft Excel
  └─ .ppt/.pptx  → Microsoft PowerPoint
```

AppleScript 模板：
```applescript
tell application "Microsoft Word"
    with timeout of 30 seconds
        set doc to open "<文件路径>"
        print out doc
        close doc saving no
    end timeout
end tell
```

#### 3.2.5 PDF 生成器

**文件**：`src-tauri/src/pdf.rs`

手写 PDF 1.4 格式，无第三方依赖。核心功能：

- **图片嵌入**：JPEG 以 DCTDecode 直接嵌入，零质量损失；非 JPEG 格式先经 `sips` 转为 JPEG
- **文本生成**：Helvetica 12pt，仅支持 Latin-1 字符集；含中文返回 Err，由调用方回退原生打印
- **PDF 结构**：单页、A4 大小，stream 压缩使用 FlateDecode

#### 3.2.6 日志系统

**文件**：`src-tauri/src/logger.rs`

- 日志文件：`app_log_dir()/printer-assistant.log`
- 大小限制：500KB，超限自动截断保留末尾
- 日志级别：`error`、`warn`、`info`、`debug`、`trace`
- 目标：同时写入文件和标准输出
- 日志标签：`print`、`build`、`office`、`frontend`、`app`、`panic`

---

## 4. 国际化

**文件**：`src/i18n.js`

支持中文和英文，通过原生菜单切换：

```
菜单 → 语言 → 简体中文 / English
  ├─ 持久化到 app_config_dir/lang.txt
  └─ emit('language-changed') 通知前端
```

前端使用 `t(key, params)` 函数获取文案，支持参数插值 `{n}`。

---

## 5. 构建与部署

### 5.1 构建流程

```bash
npm run tauri dev    # 开发模式（Vite HMR + cargo watch）
npm run tauri build  # 生产构建
```

### 5.2 构建产物

| 平台 | 产物格式 | 路径 |
|------|---------|------|
| macOS | `.app` / `.dmg` | `src-tauri/target/release/bundle/` |
| Windows | `.msi` / `.exe` | `src-tauri/target/release/bundle/` |

### 5.3 外部依赖

| 依赖 | macOS | Windows | 用途 |
|------|-------|---------|------|
| LibreOffice | 可选 | 可选 | Office 文档 → PDF |
| Microsoft Office | 可选 | — | AppleScript 静默打印 |
| WPS / Microsoft Office | — | 可选 | COM 自动化打印 |
| MuPDF | — | 必需 | PDF 打印 |

---

## 6. 数据流

```
┌──────────┐   拖拽/选择    ┌──────────┐   invoke()   ┌─────────────┐
│  用户    │ ───────────→  │  前端    │ ──────────→  │  Rust 后端  │
│          │ ←───────────  │          │ ←──────────  │             │
│          │   UI 更新      │          │   返回结果    │             │
└──────────┘               └──────────┘              └─────────────┘
                                                           │
                                                    ┌──────┴──────┐
                                                    │  converter/  │
                                                    │  to_pdf()    │
                                                    └──────┬──────┘
                                                           │ PDF
                                                    ┌──────┴──────┐
                                                    │  printer/    │
                                                    │  print_pdf() │
                                                    └──────┬──────┘
                                                           │
                                                    ┌──────┴──────┐
                                                    │  系统打印    │
                                                    │  CUPS/mutool │
                                                    └─────────────┘
```

---

## 7. 安全与错误处理

- **文件路径**：所有路径经 `std::fs::canonicalize()` 规范化
- **超时保护**：AppleScript 操作设置 30 秒超时
- **日志记录**：所有打印操作、转换失败、异常 panic 均写入日志文件
- **前端防御**：
  - 文件去重（按路径）
  - 打印中禁止操作（删除、清空、添加）
  - 打印机状态定期刷新（3 秒间隔）
  - 不支持的文件类型 toast 提示
- **回退机制**：每层优先路径失败后自动回退通用管线

---

## 8. 项目结构

```
printer/
├── index.html                  # 前端入口 HTML
├── vite.config.mjs             # Vite 构建配置
├── package.json                # Node 依赖与脚本
├── src/                        # 前端源码
│   ├── main.js                 # 主逻辑（~890 行）
│   ├── i18n.js                 # 国际化文案
│   └── styles.css              # 样式表（~850 行）
├── src-tauri/                  # Rust 后端
│   ├── tauri.conf.json         # 应用配置
│   ├── Cargo.toml              # Rust 依赖
│   ├── build.rs                # 构建脚本
│   └── src/
│       ├── main.rs             # 程序入口
│       ├── lib.rs              # 命令注册 & 打印管线（~650 行）
│       ├── pdf.rs              # PDF 生成器（~350 行）
│       ├── logger.rs           # 文件日志（~90 行）
│       ├── printer/
│       │   ├── mod.rs          # 打印分派
│       │   ├── macos.rs        # macOS: CUPS + AppleScript
│       │   └── windows.rs      # Windows: MuPDF
│       └── converter/
│           ├── mod.rs          # 转换入口
│           ├── office.rs       # LibreOffice 转换
│           ├── image.rs        # 图片转 PDF
│           └── text.rs         # 文本转 PDF
├── doc/                        # 设计文档
│   └── architecture.md         # 本文档
└── test/                       # 测试工具
    └── convert_all.sh          # officecli 批量转图脚本
```
