use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use log::info;
use tauri::{AppHandle, Emitter};

/// 下载进度事件载荷
#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
    speed_kbps: u64,
    percent: u8,
    status: String,  // "connecting" | "downloading" | "installing" | "done" | "error"
    message: String,
}

/// 获取当前平台对应的 LibreOffice 下载 URL 和文件名
fn get_download_url() -> Result<(String, String), String> {
    #[cfg(target_os = "macos")]
    {
        let arch_output = Command::new("uname")
            .arg("-m")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let (arch_path, suffix) = if arch_output == "arm64" {
            ("aarch64", "aarch64")
        } else {
            ("x86_64", "x86-64")
        };

        // 选用2024年底的稳定版；版本号可随需要更新
        let version = "24.8.3";
        let url = format!(
            "https://download.documentfoundation.org/libreoffice/stable/{version}/mac/{arch_path}/LibreOffice_{version}_MacOS_{suffix}.dmg",
            version = version,
            arch_path = arch_path,
            suffix = suffix
        );
        let filename = format!("LibreOffice_{version}_MacOS_{suffix}.dmg");
        Ok((url, filename))
    }

    #[cfg(target_os = "windows")]
    {
        let version = "24.8.3";
        let url = format!(
            "https://download.documentfoundation.org/libreoffice/stable/{version}/win/x86_64/LibreOffice_{version}_Win_x86-64.msi",
            version = version
        );
        let filename = format!("LibreOffice_{version}_Win_x86-64.msi");
        Ok((url, filename))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("当前操作系统不支持自动下载 LibreOffice".into())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// 安装下载好的 LibreOffice
#[cfg(target_os = "macos")]
fn install_libreoffice(dmg_path: &Path) -> Result<(), String> {
    info!(target: "install", "安装 LibreOffice: {}", dmg_path.display());

    // 1. 挂载 DMG
    let mount_output = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-quiet", &dmg_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("挂载 DMG 失败: {}", e))?;

    if !mount_output.status.success() {
        let stderr = String::from_utf8_lossy(&mount_output.stderr);
        return Err(format!("挂载 DMG 失败: {}", stderr));
    }

    // 2. 解析挂载点（hdiutil attach 输出的最后一行最后一个字段）
    let stdout = String::from_utf8_lossy(&mount_output.stdout);
    let mount_point = stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            // 挂载点通常是 /Volumes/xxx，取最后一个字段
            parts.last().map(|s| s.to_string())
        })
        .last()
        .ok_or_else(|| "无法解析 DMG 挂载点".to_string())?;

    info!(target: "install", "DMG 已挂载到: {}", mount_point);

    // 3. 找到 LibreOffice.app
    let mounted_app = Path::new(&mount_point).join("LibreOffice.app");
    if !mounted_app.exists() {
        // 立即卸载并返回错误
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", &mount_point])
            .output();
        return Err("DMG 中未找到 LibreOffice.app".into());
    }

    // 4. 移除旧的安装（如果有）
    let dest = Path::new("/Applications/LibreOffice.app");
    if dest.exists() {
        info!(target: "install", "移除旧版 LibreOffice");
        std::fs::remove_dir_all(dest).map_err(|e| format!("移除旧版失败: {}", e))?;
    }

    // 5. 复制到 /Applications
    info!(target: "install", "复制 LibreOffice.app 到 /Applications...");
    let cp_status = Command::new("cp")
        .args(["-R", &mounted_app.to_string_lossy(), "/Applications/"])
        .status()
        .map_err(|e| format!("复制 LibreOffice.app 失败: {}", e))?;

    if !cp_status.success() {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", &mount_point])
            .output();
        return Err("复制 LibreOffice.app 到 /Applications 失败".into());
    }

    // 6. 卸载 DMG
    let _ = Command::new("hdiutil")
        .args(["detach", "-quiet", &mount_point])
        .output();

    // 7. 清理 DMG 文件
    let _ = std::fs::remove_file(dmg_path);

    info!(target: "install", "LibreOffice 安装完成");
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_libreoffice(msi_path: &Path) -> Result<(), String> {
    info!(target: "install", "安装 LibreOffice: {}", msi_path.display());

    let status = Command::new("msiexec")
        .args([
            "/i",
            &msi_path.to_string_lossy(),
            "/quiet",
            "/norestart",
        ])
        .status()
        .map_err(|e| format!("启动安装程序失败: {}", e))?;

    if !status.success() {
        let code = status.code().map(|c| c.to_string()).unwrap_or("unknown".into());
        return Err(format!("安装失败 (退出码: {})", code));
    }

    // 安装成功后清理 MSI 文件
    let _ = std::fs::remove_file(msi_path);

    info!(target: "install", "LibreOffice 安装完成");
    Ok(())
}

/// 下载并安装 LibreOffice
/// 通过 Tauri 事件 `download-progress` 向前端推送进度
#[tauri::command]
pub async fn download_libreoffice(app: AppHandle) -> Result<String, String> {
    info!(target: "download", "开始下载 LibreOffice");

    // 获取下载 URL
    let (url, filename) = get_download_url()?;
    info!(target: "download", "下载地址: {}", url);

    let dest_dir = std::env::temp_dir().join("printer_assistant");
    let dest = dest_dir.join(&filename);
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建临时目录失败: {}", e))?;

    // 通知前端开始连接
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded: 0,
            total: 0,
            speed_kbps: 0,
            percent: 0,
            status: "connecting".into(),
            message: "正在连接下载服务器...".into(),
        },
    );

    // 创建 HTTP 客户端
    let client = reqwest::Client::builder()
        .user_agent("PrinterAssistant/1.0")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    // 发送下载请求
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("下载请求失败: {}", e);
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    downloaded: 0,
                    total: 0,
                    speed_kbps: 0,
                    percent: 0,
                    status: "error".into(),
                    message: msg.clone(),
                },
            );
            msg
        })?;

    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&dest).map_err(|e| {
        let msg = format!("创建文件失败: {}", e);
        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                downloaded: 0,
                total: 0,
                speed_kbps: 0,
                percent: 0,
                status: "error".into(),
                message: msg.clone(),
            },
        );
        msg
    })?;

    // 流式下载，每 ~200ms 发射一次进度事件
    let mut downloaded: u64 = 0;
    let start = Instant::now();
    let mut last_emit = Instant::now();

    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| {
            let msg = format!("下载中断: {}", e);
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    downloaded,
                    total,
                    speed_kbps: 0,
                    percent: if total > 0 {
                        (downloaded * 100 / total) as u8
                    } else {
                        0
                    },
                    status: "error".into(),
                    message: msg.clone(),
                },
            );
            msg
        })?;

        file.write_all(&chunk).map_err(|e| {
            let msg = format!("写入文件失败: {}", e);
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    downloaded,
                    total,
                    speed_kbps: 0,
                    percent: if total > 0 {
                        (downloaded * 100 / total) as u8
                    } else {
                        0
                    },
                    status: "error".into(),
                    message: msg.clone(),
                },
            );
            msg
        })?;

        downloaded += chunk.len() as u64;

        // 限频发射进度事件（每 200ms 或下载完成时）
        if last_emit.elapsed() >= std::time::Duration::from_millis(200) || downloaded >= total {
            let elapsed = start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                (downloaded as f64 / elapsed) as u64
            } else {
                0
            };
            let percent = if total > 0 {
                (downloaded * 100 / total) as u8
            } else {
                0
            };

            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    downloaded,
                    total,
                    speed_kbps: speed / 1024,
                    percent,
                    status: "downloading".into(),
                    message: format!(
                        "下载中... {} / {} ({} KB/s)",
                        format_size(downloaded),
                        format_size(total),
                        speed / 1024
                    ),
                },
            );
            last_emit = Instant::now();
        }
    }

    // 下载完成，通知前端开始安装
    info!(target: "download", "下载完成，开始安装");
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded,
            total,
            speed_kbps: 0,
            percent: 100,
            status: "installing".into(),
            message: "正在安装 LibreOffice...".into(),
        },
    );

    // 安装
    install_libreoffice(&dest)?;

    // 安装完成
    info!(target: "download", "LibreOffice 安装完成");
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded,
            total,
            speed_kbps: 0,
            percent: 100,
            status: "done".into(),
            message: "LibreOffice 安装完成".into(),
        },
    );

    Ok("LibreOffice 安装完成".into())
}
