// WKWebView 路径暂停使用，代码保留供后续调试
#![allow(dead_code)]

/// HTML → PDF 转换引擎
///
/// macOS 用常驻 WKWebView 渲染：
///   loadFileURL → 等渲染完成（readyState + images + fonts）
///   → 注入 viewport meta（width=595 强制 A4 视口）
///   → evaluateJavaScript 拿真实 scrollHeight → setFrame 到 (595, h)
///   → createPDF（CGRectNull = 全量截取为 1 页）
///   → CoreGraphics 把单页长 PDF 切 A4 多页（正向分页避免重叠）
///
/// 回调通道用全局静态 OnceLock<Mutex<Option<Sender>>>，block 整体堆上 + 'static，
/// 避免原来栈上 `_NSConcreteStackBlock` 出栈即失效的悬垂引用。

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::ffi::{c_void, CStr};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;

use crate::converter::{office, unique_name};
#[cfg(target_os = "macos")]
use crate::APP_HANDLE;

// ── A4 尺寸常量 ────────────────────────────────────
#[cfg(target_os = "macos")]
const A4_PT_W: f64 = 595.0;  // A4 宽（PDF 点 @72dpi）
#[cfg(target_os = "macos")]
const A4_PT_H: f64 = 842.0;  // A4 高（PDF 点 @72dpi）

// ── 单例 WKWebView ──────────────────────────────────

#[cfg(target_os = "macos")]
static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

#[cfg(target_os = "macos")]
struct Engine {
    window: *mut objc2::runtime::NSObject,
    webview: *mut objc2::runtime::NSObject,
}

#[cfg(target_os = "macos")]
unsafe impl Send for Engine {}
#[cfg(target_os = "macos")]
unsafe impl Sync for Engine {}

#[cfg(target_os = "macos")]
pub fn init_print_engine() -> Result<(), String> {
    use objc2::msg_send;
    use objc2::runtime::NSObject;

    let (window, webview) = objc2::rc::autoreleasepool(|_| -> Result<(_, _), String> {
        let cls = |name: &str| -> Result<&'static objc2::runtime::AnyClass, String> {
            let s = format!("{}\0", name);
            let c = CStr::from_bytes_with_nul(s.as_bytes()).map_err(|_| "CStr")?;
            objc2::runtime::AnyClass::get(c).ok_or_else(|| format!("{} not found", name))
        };
        // 窗口放屏幕可见区域内 (0,0)。之前离屏 → macOS 跳过瓦片渲染
        // alpha=0（全透明）也可能被优化跳过。这里先用可见位置测试。
        let off = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(A4_PT_W, A4_PT_H),
        );
        Ok(unsafe {
            let win: *mut NSObject = msg_send![cls("NSWindow")?, alloc];
            let win: *mut NSObject = msg_send![win, initWithContentRect: off, styleMask: 0, backing: 2, defer: false];
            let cfg: *mut NSObject = msg_send![cls("WKWebViewConfiguration")?, new];
            let wv: *mut NSObject = msg_send![cls("WKWebView")?, alloc];
            let wv: *mut NSObject = msg_send![wv, initWithFrame: off, configuration: cfg];
            let _: () = msg_send![win, setContentView: wv];
            // 窗口初始保持 hidden（Tauri setup 阶段 runloop 未就绪，visible 会导致崩溃）。
            // do_export 在 createPDF 前才设 setIsVisible:true 触发 render loop。
            (win, wv)
        })
    }).map_err(|e: String| e)?;

    ENGINE.set(Mutex::new(Engine { window, webview })).map_err(|_| "ENGINE 已初始化".into())
}

#[cfg(not(target_os = "macos"))]
pub fn init_print_engine() -> Result<(), String> { Ok(()) }

// ── 转换入口 ───────────────────────────────────────

/// 判断当前平台是否有可用的 HTML→PDF 渲染引擎。
/// - macOS：内置 WKWebView，始终可用。
/// - 其它：优先 Chrome headless，其次 LibreOffice / Fulgur（Fulgur 纯 Rust 始终可用）。
pub fn html_engine_available() -> bool {
    #[cfg(target_os = "macos")]
    { true }

    #[cfg(not(target_os = "macos"))]
    { find_chrome().is_some() || office::libreoffice_available() }
}

pub fn html_to_pdf(input: &Path, tmp: &Path) -> Result<PathBuf, String> {
    // 1. Chrome headless（渲染最稳定，支持 JS 图表）
    let out = tmp.join(unique_name("html", "pdf"));
    if chrome_to_pdf(input, &out).is_ok() && out.exists() { return Ok(out); }

    // 2. macOS textutil（Safari/WebKit 内核，zero 额外依赖）
    #[cfg(target_os = "macos")]
    { let out = tmp.join(unique_name("html", "pdf"));
      if textutil_to_pdf(input, &out).is_ok() && out.exists() { return Ok(out); }}

    // 3. Fulgur（纯 Rust 引擎，无 JS，快速静态兜底）
    let out = tmp.join(unique_name("html", "pdf"));
    if fulgur_to_pdf(input, &out).is_ok() && out.exists() { return Ok(out); }

    // 4. LibreOffice（最终兜底）
    office::libreoffice_to_pdf(input, tmp)
}

// ── WKWebView 转换（macOS 专属） ────────────────────

#[cfg(target_os = "macos")]
type PdfResult = Result<Vec<u8>, String>;
#[cfg(target_os = "macos")]
type JsResult = Result<String, String>;

// 全局回调通道（单例 WKWebView 串行使用，不会有并发竞争）
#[cfg(target_os = "macos")]
static PDF_SLOT: OnceLock<Mutex<Option<mpsc::Sender<PdfResult>>>> = OnceLock::new();
#[cfg(target_os = "macos")]
static JS_SLOT: OnceLock<Mutex<Option<mpsc::Sender<JsResult>>>> = OnceLock::new();

// ── 手动构造的 ObjC block（避开栈上悬垂） ──────────────

#[cfg(target_os = "macos")]
#[repr(C)]
struct PDesc { reserved: usize, size: usize }

#[cfg(target_os = "macos")]
#[repr(C)]
struct PBlock {
    isa: *mut c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*mut c_void, *mut objc2::runtime::NSObject, *mut objc2::runtime::NSObject),
    descriptor: *mut PDesc,
    ctx: *mut c_void,
}

/// 把 block 整个 heap 化 + 'static，isa 用 `_NSConcreteGlobalBlock`
/// （全局 block 类，'static 生命周期，永远不悬垂）
#[cfg(target_os = "macos")]
fn make_block(
    ctx: *mut c_void,
    invoke: unsafe extern "C" fn(*mut c_void, *mut objc2::runtime::NSObject, *mut objc2::runtime::NSObject),
) -> *mut objc2::runtime::NSObject {
    use objc2::runtime::NSObject;
    unsafe {
        extern "C" { static _NSConcreteGlobalBlock: *mut c_void; }
        let desc = Box::leak(Box::new(PDesc { reserved: 0, size: std::mem::size_of::<PBlock>() }));
        let block = Box::leak(Box::new(PBlock {
            isa: _NSConcreteGlobalBlock,
            flags: 0,
            reserved: 0,
            invoke,
            descriptor: desc as *mut PDesc,
            ctx,
        }));
        block as *mut PBlock as *mut NSObject
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn pdf_invoke(b: *mut c_void, data: *mut objc2::runtime::NSObject, _err: *mut objc2::runtime::NSObject) {
    use objc2::msg_send;
    // Apple Block ABI：invoke 第一个参数是 block 自身，ctx 在 block 的 ctx 字段
    // 之前 bug：把 block 自身当 ctx deref → 野指针 Mutex → pthread EINVAL
    let pblock = &*(b as *const PBlock);
    let cell = &*(pblock.ctx as *const Mutex<Option<mpsc::Sender<PdfResult>>>);
    if let Some(tx) = cell.lock().unwrap().take() {
        if !data.is_null() {
            let ptr: *const u8 = msg_send![data, bytes];
            let len: u64 = msg_send![data, length];
            let bytes = std::slice::from_raw_parts(ptr, len as usize);
            let _ = tx.send(Ok(bytes.to_vec()));
        } else {
            let _ = tx.send(Err("createPDF 返回空数据".into()));
        }
    }
    // _err 走的是 data 通道（createPDF 的 NSData 成功时 err 必为 null，失败时 data 为 null）
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn js_invoke(b: *mut c_void, result: *mut objc2::runtime::NSObject, err: *mut objc2::runtime::NSObject) {
    use objc2::msg_send;
    // Apple Block ABI：invoke 第一个参数是 block 自身，ctx 在 block 的 ctx 字段
    let pblock = &*(b as *const PBlock);
    let cell = &*(pblock.ctx as *const Mutex<Option<mpsc::Sender<JsResult>>>);
    if let Some(tx) = cell.lock().unwrap().take() {
        if !err.is_null() {
            let s: *mut objc2_foundation::NSString = msg_send![err, localizedDescription];
            let utf8: *const i8 = msg_send![s, UTF8String];
            let cstr = CStr::from_ptr(utf8);
            let _ = tx.send(Err(format!("JS 错误: {}", cstr.to_string_lossy())));
        } else if !result.is_null() {
            // result 类型不定（NSString / NSNumber / NSNull），统一 description 取字符串
            let s: *mut objc2_foundation::NSString = msg_send![result, description];
            let utf8: *const i8 = msg_send![s, UTF8String];
            let cstr = CStr::from_ptr(utf8);
            let _ = tx.send(Ok(cstr.to_string_lossy().into_owned()));
        } else {
            let _ = tx.send(Ok(String::new()));
        }
    }
}

// ── 评估 JS（主线程内调用，必须在主线程跑） ─────────

#[cfg(target_os = "macos")]
fn eval_js(wv: *mut objc2::runtime::NSObject, script: &str) -> Result<String, String> {
    use objc2::msg_send;
    let (tx, rx) = mpsc::channel();
    JS_SLOT.get_or_init(|| Mutex::new(None)).lock().unwrap().replace(tx);
    let script_ns = objc2_foundation::NSString::from_str(script);
    let ctx_ptr = JS_SLOT.get().unwrap() as *const Mutex<Option<mpsc::Sender<JsResult>>> as *mut c_void;
    let block = make_block(ctx_ptr, js_invoke);
    let preview: String = script.chars().take(60).collect();
    log::info!(target: "html_webview", "eval_js: {}", preview);
    let _: () = unsafe { msg_send![wv, evaluateJavaScript: &*script_ns, completionHandler: block] };

    // 用 NSRunLoop 主循环等回调：block 在某次 runloop 派发时触发，回调 take tx，rx 收到
    let rl = objc2_foundation::NSRunLoop::currentRunLoop();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            log::warn!(target: "html_webview", "eval_js 超时（2s 未收到回调）");
            return Err("evaluateJavaScript 超时".to_string());
        }
        let until_cls = objc2::runtime::AnyClass::get(c"NSDate").unwrap();
        let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.020] };
        let _: () = unsafe { msg_send![&rl, runUntilDate: until] };
        if let Ok(r) = rx.try_recv() { return r; }
    }
}

#[cfg(target_os = "macos")]
fn wkwebview_to_pdf(html_path: &Path, output: &Path) -> Result<(), String> {
    let input_str = html_path.to_str().ok_or("路径非 UTF-8")?;
    let out_str = output.to_str().ok_or("路径非 UTF-8")?;
    let input_owned = input_str.to_string();
    let out_owned = out_str.to_string();
    let (tx, rx) = mpsc::channel();

    let app = APP_HANDLE.get().ok_or("AppHandle 未初始化")?;
    app.run_on_main_thread(move || {
        let engine = ENGINE.get().expect("ENGINE").lock().unwrap();
        let wv = engine.webview;
        let win = engine.window;
        let r = do_export(wv, win, &input_owned, &out_owned);
        let _ = tx.send(r);
    }).map_err(|e| format!("调度主线程失败: {}", e))?;

    rx.recv_timeout(Duration::from_secs(120)).map_err(|_| "引擎超时".to_string())?
}

#[cfg(target_os = "macos")]
fn do_export(wv: *mut objc2::runtime::NSObject, win: *mut objc2::runtime::NSObject,
    input_owned: &str, out_owned: &str) -> Result<(), String> {
    use objc2::msg_send;
    use objc2::runtime::NSObject;

    log::info!(target: "html_webview", "do_export 开始: {}", input_owned);

    // 激活隐藏窗口：放在屏幕内 (0,0)，极低透明度（非 0，防止 macOS 跳过渲染）
    // 不抢焦点、不响应鼠标，用户无感知。WkWebView 在 visible 窗口下跑 render loop → 瓦片全部渲染
    unsafe {
        let _: () = msg_send![win, setAlphaValue: 0.005_f64];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setIsVisible: true];
    }

    objc2::rc::autoreleasepool(|_| -> Result<(), String> {
        let cls = |name: &str| -> Result<&'static objc2::runtime::AnyClass, String> {
            let s = format!("{}\0", name);
            let c = CStr::from_bytes_with_nul(s.as_bytes()).map_err(|_| "CStr".to_string())?;
            objc2::runtime::AnyClass::get(c).ok_or_else(|| format!("{} not found", name))
        };

        // 1. 初始画布：A4 点尺寸（595pt = 210mm @72dpi），WKWebView 1 CSS px → 1 PDF pt
        //    createPDF 直接输出 A4 宽度，无需缩放
        let a4_w = A4_PT_W;
        let a4_h = A4_PT_H;
        // webview frame 原点用 (0,0) 相对窗口，窗口已在 (2000,-2000) 屏幕坐标
        // 这样 webview 实际在屏幕 (2000,-2000)，距屏幕足够近使 WindowServer 分配 GPU 瓦片
        let canvas = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(a4_w, a4_h),
        );
        unsafe { let _: () = msg_send![wv, setFrame: canvas]; }

        // 2. 加载 HTML
        let url_ns = objc2_foundation::NSString::from_str(input_owned);
        let url: *mut NSObject = unsafe { msg_send![cls("NSURL")?, fileURLWithPath: &*url_ns] };
        let _: *mut NSObject = unsafe { msg_send![wv, loadFileURL: url, allowingReadAccessToURL: url] };
        log::info!(target: "html_webview", "loadFileURL 已发出");

        // 3. 等渲染完成：轮询 readyState + images.complete + fonts.status
        let rl = objc2_foundation::NSRunLoop::currentRunLoop();
        let check_script = r#"
            (function(){
                try {
                    if (document.readyState !== 'complete') return 'rs';
                    if (document.images && ![...document.images].every(function(i){return i.complete;})) return 'imgs';
                    if (document.fonts && document.fonts.status !== 'loaded') return 'fonts';
                    return 'ok';
                } catch(e) { return 'err:' + e.message; }
            })()
        "#;
        let mut ready = false;
        let mut consecutive_fail = 0u32;  // 连续 eval_js 失败计数
        for tick in 0..600 {
            // 让 runloop 跑一轮（处理 ObjC pending events）—— 关键：必须给 ObjC 时间派发回调
            let until_cls = objc2::runtime::AnyClass::get(c"NSDate").unwrap();
            let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.05] };
            let _: () = unsafe { msg_send![&rl, runUntilDate: until] };

            // 起步后给 5 tick 初始化时间（避免太早查询 document 不存在）
            if tick < 5 { continue; }

            match eval_js(wv, check_script) {
                Ok(s) if s == "ok" => {
                    ready = true;
                    log::info!(target: "html_webview", "渲染就绪 (tick={})", tick);
                    break;
                }
                Ok(s) => {
                    // 还在等资源：rs / imgs / fonts
                    consecutive_fail = 0;
                    if tick % 50 == 0 { log::info!(target: "html_webview", "等渲染: status={} (tick={})", s, tick); }
                }
                Err(_) => {
                    consecutive_fail += 1;
                    if consecutive_fail >= 3 {
                        // eval_js block 回调没触发（WKWebView 可能没在主线程跑 evaluateJavaScript）
                        log::error!(target: "html_webview", "eval_js 连续 {} 次无回调，放弃 WKWebView 路径，走 chrome 兜底", consecutive_fail);
                        return Err("WKWebView 评估 JS 无响应".into());
                    }
                }
            }
        }
        if !ready {
            log::warn!(target: "html_webview", "渲染等待 60s 仍未就绪，继续执行");
        }

        // 4. 注入 viewport meta 确保视口宽度匹配 A4（如 HTML 已有则覆盖，避免默认 wider 视口导致缩小）
        let _ = eval_js(wv, r#"
            (function(){
                try {
                    var m = document.querySelector('meta[name="viewport"]');
                    if (m) { m.content = 'width=595'; }
                    else {
                        m = document.createElement('meta');
                        m.name = 'viewport';
                        m.content = 'width=595';
                        document.head.appendChild(m);
                    }
                    void document.documentElement.offsetHeight;
                    return 'ok';
                } catch(e) { return 'err:' + e.message; }
            })()
        "#);

        // 5. 让 webview 自己决定 layout 高度（自适应）
        let h_str = eval_js(wv, "String(document.documentElement.scrollHeight)")
            .map_err(|e| { log::error!(target: "html_webview", "拿高度失败: {}", e); e })?;
        let h: f64 = h_str.parse().map_err(|_| format!("解析高度失败: {}", h_str))?;
        let real_h = h.max(a4_h);
        log::info!(target: "html_webview", "内容高度={:.0}", h);

        // 6. 强制全画布瓦片渲染
        //    关键：KEEP frame 为 A4 高度，滚动才有效 → 触发 WKWebView 渲染远处瓦片
        //    如果提前 setFrame 到全高，文档无溢出 → scrollTo 空操作 → 远处瓦片从不渲染
        let until_cls = objc2::runtime::AnyClass::get(c"NSDate").unwrap();
        let step = a4_h;
        let steps = ((h / step).ceil() as i32).max(1);
        log::info!(target: "html_webview", "瓦片渲染: total_h={} step={} steps={}", h, step, steps);
        for i in 0..steps {
            let y = ((i as f64) * step).min(h - 1.0);
            let _ = eval_js(wv, &format!("window.scrollTo(0, {}); void document.documentElement.offsetHeight;", y));
            // 每步 200ms 让 WebKit 渲染瓦片
            for _ in 0..10 {
                let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.02] };
                let _: () = unsafe { msg_send![&rl, runUntilDate: until] };
            }
        }
        // 滚回顶部
        let _ = eval_js(wv, "window.scrollTo(0, 0); void document.documentElement.offsetHeight;");
        for _ in 0..10 {
            let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.05] };
            let _: () = unsafe { msg_send![&rl, runUntilDate: until] };
        }

        // 7. 瓦片渲染完毕，再把 frame 拉到全高供 createPDF 完整截图
        log::info!(target: "html_webview", "frame→({:.0}, {:.0})", a4_w, real_h);
        let canvas = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(a4_w, real_h),
        );
        unsafe { let _: () = msg_send![wv, setFrame: canvas]; }

        // 8. 等 layout 生效
        for _ in 0..20 {
            let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.05] };
            let _: () = unsafe { msg_send![&rl, runUntilDate: until] };
        }

        // 9. createPDF
        log::info!(target: "html_webview", "createPDF 准备");
        let pdf_cfg: *mut NSObject = unsafe { msg_send![cls("WKPDFConfiguration")?, new] };
        let (tx, rx) = mpsc::channel();
        PDF_SLOT.get_or_init(|| Mutex::new(None)).lock().unwrap().replace(tx);
        let ctx_ptr = PDF_SLOT.get().unwrap() as *const Mutex<Option<mpsc::Sender<PdfResult>>> as *mut c_void;
        let block = make_block(ctx_ptr, pdf_invoke);
        let _: () = unsafe { msg_send![wv, createPDFWithConfiguration: pdf_cfg, completionHandler: block] };
        log::info!(target: "html_webview", "createPDF 已发出");

        // 8. 等 createPDF 回调
        let raw_data: Vec<u8> = {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let until_cls = objc2::runtime::AnyClass::get(c"NSDate").unwrap();
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    log::error!(target: "html_webview", "createPDF 30s 无回调");
                    return Err("createPDF 超时".into());
                }
                let until: *mut objc2::runtime::NSObject = unsafe { msg_send![until_cls, dateWithTimeIntervalSinceNow: 0.05] };
                let _: () = unsafe { msg_send![&rl, runUntilDate: until] };
                if let Ok(r) = rx.try_recv() { break r.map_err(|e| { log::error!(target: "html_webview", "createPDF 回调 Err: {}", e); e })?; }
            }
        };
        log::info!(target: "html_webview", "createPDF 收到: {} 字节", raw_data.len());

        // 9. split_pdf
        log::info!(target: "html_webview", "split_pdf 开始: out={}", out_owned);
        split_pdf(&raw_data, out_owned)
    })
}

/// CoreGraphics 将单页长 PDF 按 A4 高度切多页
///
/// **PDF 坐标系 y 向上**：`y=0` 是 src 底部，`y=total_h` 是 src 顶部。
/// 反向分页：i=0 画 src 顶部（HTML 开头），i=last 画 src 底部。
/// 完整页用平移切，剩余页（< 一整页）用 CGContextClipToRect 限制画布显示范围——避免重叠。
#[cfg(target_os = "macos")]
fn split_pdf(raw: &[u8], out_path: &str) -> Result<(), String> {
    extern "C" {
        fn CGPDFDocumentCreateWithProvider(p: *const c_void) -> *mut c_void;
        fn CGPDFDocumentGetNumberOfPages(doc: *mut c_void) -> i32;
        fn CGDataProviderCreateWithCFData(d: *const c_void) -> *mut c_void;
        fn CGPDFDocumentGetPage(doc: *mut c_void, n: i32) -> *mut c_void;
        fn CGPDFPageGetBoxRect(page: *mut c_void, r#box: i32) -> objc2_foundation::NSRect;
        fn CFURLCreateFromFileSystemRepresentation(a: *const c_void, p: *const i8, l: isize, d: bool) -> *mut c_void;
        fn CGPDFContextCreateWithURL(u: *mut c_void, m: *const objc2_foundation::NSRect, a: *const c_void) -> *mut c_void;
        fn CGContextBeginPage(c: *mut c_void, m: *const objc2_foundation::NSRect);
        fn CGContextTranslateCTM(c: *mut c_void, tx: f64, ty: f64);
        fn CGContextDrawPDFPage(c: *mut c_void, p: *mut c_void);
        fn CGContextClipToRect(c: *mut c_void, r: *const objc2_foundation::NSRect);
        fn CGContextEndPage(c: *mut c_void);
        fn CGPDFContextClose(c: *mut c_void);
        fn CFRelease(v: *const c_void);
    }

    unsafe {
        // 读源 PDF
        let cfdata = objc2_foundation::NSData::with_bytes(raw);
        let provider = CGDataProviderCreateWithCFData(&*cfdata as *const _ as *const c_void);
        let src_doc = CGPDFDocumentCreateWithProvider(provider);
        if src_doc.is_null() { return Err("CGPDFDocument 创建失败".into()); }

        let page_count = CGPDFDocumentGetNumberOfPages(src_doc);
        log::info!(target: "html_webview", "split_pdf: 源 PDF 共 {} 页", page_count);

        let a4_w = A4_PT_W;
        let a4_h = A4_PT_H;

        // 创建输出 PDF 上下文
        let out_cstr = std::ffi::CString::new(out_path).map_err(|_| "路径 CString 失败")?;
        let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), out_cstr.as_ptr(), out_path.len() as isize, false);
        if url.is_null() {
            CFRelease(src_doc as *const c_void);
            CFRelease(provider as *const c_void);
            return Err("URL 创建失败".into());
        }

        let media = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(a4_w, a4_h),
        );
        let ctx = CGPDFContextCreateWithURL(url, &media, std::ptr::null());
        if ctx.is_null() {
            CFRelease(url as *const c_void);
            CFRelease(src_doc as *const c_void);
            CFRelease(provider as *const c_void);
            return Err("CGContext 创建失败".into());
        }

        // 遍历源 PDF 的每一页，按 A4 高度切分输出
        let mut output_page_num = 0;
        for src_page_num in 1..=page_count {
            let src_page = CGPDFDocumentGetPage(src_doc, src_page_num);
            if src_page.is_null() { continue; }

            let page_rect = CGPDFPageGetBoxRect(src_page, 0); // kCGPDFMediaBox = 0
            let total_h = page_rect.size.height;
            let full_pages = (total_h / a4_h).floor() as i32;
            let remainder = total_h - full_pages as f64 * a4_h;

            log::info!(target: "html_webview",
                "split_pdf: 源页#{}/{} h={:.0} full={} rem={:.0}",
                src_page_num, page_count, total_h, full_pages, remainder);

            if total_h <= 0.0 { continue; }

            // 反向分页：i=0 画顶部，i=last 画底部
            for i in 0..full_pages {
                CGContextBeginPage(ctx, &media);
                let start = total_h - (i as f64 + 1.0) * a4_h;
                CGContextTranslateCTM(ctx, 0.0, -start);
                CGContextDrawPDFPage(ctx, src_page);
                CGContextEndPage(ctx);
                output_page_num += 1;
            }

            // 剩余不足一页的部分
            if remainder > a4_h * 0.05 {
                CGContextBeginPage(ctx, &media);
                let clip = objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(a4_w, remainder),
                );
                CGContextClipToRect(ctx, &clip);
                CGContextTranslateCTM(ctx, 0.0, 0.0);
                CGContextDrawPDFPage(ctx, src_page);
                CGContextEndPage(ctx);
                output_page_num += 1;
            }
        }

        log::info!(target: "html_webview", "split_pdf: 输出 {} 页", output_page_num);
        CGPDFContextClose(ctx);
        CFRelease(ctx as *const c_void);
        CFRelease(url as *const c_void);
        CFRelease(src_doc as *const c_void);
        CFRelease(provider as *const c_void);
    }
    if Path::new(out_path).exists() { Ok(()) } else { Err("分页 PDF 未生成".into()) }
}

// ── Fulgur（纯 Rust HTML→PDF） ─────────────────────

/// 用 fulgur 引擎将 HTML 渲染为 PDF（零 WebView/Chrome 依赖，自动分页）
fn fulgur_to_pdf(input: &Path, output: &Path) -> Result<PathBuf, String> {
    use std::path::Path;

    let html = std::fs::read_to_string(input).map_err(|e| format!("读取 HTML 失败: {}", e))?;
    let base = input.parent().unwrap_or(Path::new("."));

    let engine = fulgur::engine::Engine::builder()
        .page_size(fulgur::config::PageSize::A4)
        .margin(fulgur::config::Margin::uniform_mm(10.0))
        .system_fonts(true)
        .base_path(base)
        .build();

    let pdf_bytes = engine
        .render(&html)
        .map_err(|e| format!("fulgur 渲染失败: {:?}", e))?;

    std::fs::write(output, &pdf_bytes).map_err(|e| format!("写入 PDF 失败: {}", e))?;

    if output.exists() {
        log::info!(target: "html_webview", "fulgur 成功: {}", output.display());
        Ok(output.to_path_buf())
    } else {
        Err("fulgur 未生成文件".into())
    }
}

// ── 兜底 ─────────────────────────────────────────────

/// Chrome headless（备选渲染：需要本机装 Chrome，新版强制 --headless=new）
///
/// 关键点（经实测验证，避免弹出可见 Chrome 窗口）：
/// - 必须用**每次全新**的独立 `--user-data-dir`，且放在系统 temp 而非用户文档目录：
///   若已有普通 Chrome 实例在运行，headless 新进程会被委托给现有实例（不执行 headless，
///   甚至弹出标签页窗口）；若 user-data-dir 内残留 SingletonLock/SingletonCookie 等，
///   Chrome 会误判已有实例而弹窗。
/// - 必须加 `--no-first-run` + `--no-default-browser-check` + `--disable-default-apps` 等，
///   否则全新 profile 首次初始化会加载 chrome://newtab/ 标签页并弹出可见窗口。
/// - 用 `file:///` URL 而非裸文件路径，确保加载的是本地文件内容。
/// - 加 `--virtual-time-budget` 等待 JS / 字体渲染完成，避免空白页。
fn chrome_to_pdf(input: &Path, output: &Path) -> Result<PathBuf, String> {
    let chrome = find_chrome().ok_or("未找到 Chrome")?;
    // 打印输出必须用 = 绑在 flag 上，否则 URL + 单独 .arg(output) 会被解释为多个 target
    let print_flag = format!("--print-to-pdf={}", output.display());

    // 独立且每次全新的 user-data-dir，放在系统 temp，避免污染用户文档目录；
    // 每次调用用唯一名，确保 profile 目录不含残留的 SingletonLock/Cookie/Socket。
    let profile_dir = std::env::temp_dir().join(unique_name("chrome_profile", ""));
    let profile_flag = format!("--user-data-dir={}", profile_dir.display());

    // 本地文件用 file:/// URL（将 \ 统一为 /）
    let file_url = {
        let p = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
        let normalized = p.to_string_lossy().replace('\\', "/");
        format!("file:///{}", normalized)
    };

    let s = std::process::Command::new(&chrome)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-default-apps",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-sync",
            "--disable-popup-blocking",
            "--no-pings",
            "--disable-features=Translate",
            "--run-all-compositor-stages-before-draw",
            "--no-pdf-header-footer",
            // 让 JS/字体有时间渲染
            "--virtual-time-budget=10000",
            &profile_flag,
            &print_flag,
        ])
        .arg(&file_url) // URL 必须是唯一的位置参数
        .status()
        .map_err(|e| format!("Chrome: {}", e))?;

    // 清理临时 profile 目录（含 SingletonLock 等残留，避免下次误判实例）
    let _ = std::fs::remove_dir_all(&profile_dir);

    // 校验：非空且合理大小（<8KB 多半是空白/空壳 PDF）
    let ok = s.success()
        && output.exists()
        && output.metadata().map(|m| m.len() >= 8 * 1024).unwrap_or(false);
    if ok {
        Ok(output.to_path_buf())
    } else {
        let sz = output.metadata().map(|m| m.len()).unwrap_or(0);
        log::warn!(target: "html_webview", "Chrome headless 转换失败或输出过小: exit={} size={}", s.code().unwrap_or(-1), sz);
        Err("Chrome 转换失败或输出异常".into())
    }
}

fn find_chrome() -> Option<String> {
    let c: &[&str] = if cfg!(target_os="macos") { &["google-chrome","/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"] }
        else if cfg!(target_os="windows") { &["chrome",r"C:\Program Files\Google\Chrome\Application\chrome.exe"] }
        else { &["google-chrome"] };
    for p in c {
        if p.contains('/') { if Path::new(p).is_file() { return Some(p.to_string()); } }
        else if std::process::Command::new(p).arg("--version").output().map(|o|o.status.success()).unwrap_or(false) { return Some(p.to_string()); }
    }
    None
}

#[cfg(target_os = "macos")]
fn textutil_to_pdf(input: &Path, output: &Path) -> Result<PathBuf, String> {
    let s = std::process::Command::new("textutil")
        .args(["-convert", "pdf", "-output"]).arg(output).arg(input)
        .status().map_err(|e| format!("textutil: {}", e))?;
    if s.success() && output.exists() { Ok(output.to_path_buf()) } else { Err("textutil 失败".into()) }
}
