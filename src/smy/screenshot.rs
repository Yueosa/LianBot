use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::debug;

// ── HTML → JPEG → base64 ────────────────────────────────────────────────────

/// 将 HTML 字符串渲染为截图，返回 JPEG base64 编码。
/// 依赖系统命令 `wkhtmltoimage`（apt install wkhtmltopdf）。
pub async fn capture(html: &str) -> Result<String> {
    let html_owned = html.to_string();
    tokio::task::spawn_blocking(move || capture_sync(&html_owned))
        .await
        .context("截图任务 panic")?
}

fn capture_sync(html: &str) -> Result<String> {
    use std::process::Command;

    // 生成唯一临时文件路径
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let html_path = format!("/tmp/lianbot_smy_{ts}.html");
    let img_path  = format!("/tmp/lianbot_smy_{ts}.jpg");

    std::fs::write(&html_path, html).context("写入临时 HTML 失败")?;

    // 调用 wkhtmltoimage 渲染
    let output = Command::new("wkhtmltoimage")
        .args([
            "--quiet",
            "--format",         "jpg",
            "--quality",        "85",
            "--width",          "1200",
            "--disable-smart-width",
            "--enable-local-file-access",
            &html_path,
            &img_path,
        ])
        .output()
        .context("启动 wkhtmltoimage 失败，请确认已安装: apt install wkhtmltopdf")?;

    // 清理 HTML 临时文件
    let _ = std::fs::remove_file(&html_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("wkhtmltoimage 失败: {stderr}");
    }

    let img_data = std::fs::read(&img_path).context("读取截图文件失败")?;
    let _ = std::fs::remove_file(&img_path);

    let size_kb = img_data.len() / 1024;
    debug!("截图完成: {size_kb}KB JPEG");

    Ok(B64.encode(&img_data))
}
