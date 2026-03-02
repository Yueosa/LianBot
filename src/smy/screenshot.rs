use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::debug;

// ── HTML → PNG → base64 ─────────────────────────────────────────────────────

/// 将 HTML 字符串渲染为截图，返回 PNG base64 编码。
/// 依赖系统安装的 Chrome / Chromium（通过 CLI `--screenshot` 模式）。
pub async fn capture(html: &str) -> Result<String> {
    let html_owned = html.to_string();
    tokio::task::spawn_blocking(move || capture_sync(&html_owned))
        .await
        .context("截图任务 panic")?
}

fn capture_sync(html: &str) -> Result<String> {
    use std::process::Command;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let html_path = format!("/tmp/lianbot_smy_{ts}.html");
    let img_path  = format!("/tmp/lianbot_smy_{ts}.png");

    std::fs::write(&html_path, html).context("写入临时 HTML 失败")?;

    let chrome = find_chrome()?;
    debug!("使用 Chrome: {chrome}");

    let output = Command::new(&chrome)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--hide-scrollbars",
            "--run-all-compositor-stages-before-draw",
            "--virtual-time-budget=5000",
            "--user-data-dir=/tmp/lianbot-chrome",
            "--crash-dumps-dir=/tmp/lianbot-chrome-crashes",
            "--disable-breakpad",
            &format!("--screenshot={img_path}"),
            "--window-size=1200,10000",
            &format!("file://{html_path}"),
        ])
        .output()
        .context("启动 Chrome 失败，请确认已安装 google-chrome-stable 或 chromium")?;

    // 清理 HTML 临时文件
    let _ = std::fs::remove_file(&html_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Chrome 截图失败 (exit={}): {stderr}", output.status);
    }

    let img_data = std::fs::read(&img_path).context("读取截图文件失败")?;
    let _ = std::fs::remove_file(&img_path);

    let size_kb = img_data.len() / 1024;
    debug!("截图完成: {size_kb}KB PNG");

    Ok(B64.encode(&img_data))
}

/// 查找系统中的 Chrome / Chromium 可执行文件
fn find_chrome() -> Result<String> {
    let candidates = [
        "google-chrome-stable",
        "google-chrome",
        "chromium-browser",
        "chromium",
    ];
    for name in &candidates {
        if std::process::Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(name.to_string());
        }
    }
    bail!("未找到 Chrome/Chromium，请安装: apt install google-chrome-stable")
}
