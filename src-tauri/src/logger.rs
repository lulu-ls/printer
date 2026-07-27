use chrono::Local;
use log::{LevelFilter, Metadata, Record};
use tauri::Manager;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// 日志文件上限：500 KB。超过时丢弃旧内容，仅保留末尾 MAX_SIZE 字节。
const MAX_SIZE: u64 = 500 * 1024;

/// 同时写入「系统日志目录文件」与「标准输出」的简单日志器。
/// 文件超过 MAX_SIZE 时，自动截断为末尾 MAX_SIZE 字节（即丢弃最旧的日志）。
pub struct FileLogger {
    file: Mutex<File>,
}

impl FileLogger {
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { file: Mutex::new(file) })
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let line = format!(
            "[{}][{:>5}][{}] {}\n",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            record.level(),
            record.target(),
            record.args()
        );

        // 镜像到标准输出（开发期终端可见）
        print!("{}", line);

        if let Ok(mut f) = self.file.lock() {
            // 超过上限：读取全部内容，仅保留末尾 MAX_SIZE 字节后重写
            if let Ok(meta) = f.metadata() {
                if meta.len() > MAX_SIZE {
                    let mut buf = Vec::new();
                    if f.read_to_end(&mut buf).is_ok() {
                        let keep = buf.len().saturating_sub(MAX_SIZE as usize);
                        let _ = f.seek(SeekFrom::Start(0));
                        let _ = f.set_len(0);
                        let _ = f.write_all(&buf[keep..]);
                    }
                }
            }
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// 初始化全局日志器（应在 setup 中、任何日志输出之前调用一次）。
pub fn init(app: &tauri::AppHandle) {
    let dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let path = dir.join("printer-assistant.log");

    match FileLogger::new(path) {
        Ok(logger) => {
            if log::set_boxed_logger(Box::new(logger)).is_ok() {
                log::set_max_level(LevelFilter::Info);
            }
        }
        Err(e) => {
            eprintln!("初始化日志失败: {}", e);
        }
    }
}
