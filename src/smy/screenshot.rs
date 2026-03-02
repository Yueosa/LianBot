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

    // 注入 JS：将实际内容高度写入 <title>，供第一步测量
    let patched_html = html.replace(
        "</body>",
        "<script>document.title=document.documentElement.scrollHeight;</script></body>",
    );
    std::fs::write(&html_path, &patched_html).context("写入临时 HTML 失败")?;

    let chrome = find_chrome()?;
    debug!("使用 Chrome: {chrome}");

    let _ = std::fs::create_dir_all("/tmp/lianbot-chrome");

    let base_args: Vec<&str> = vec![
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--user-data-dir=/tmp/lianbot-chrome",
        "--crash-dumps-dir=/tmp/lianbot-chrome-crashes",
        "--disable-breakpad",
    ];

    // ── 第一步：dump-dom 测量内容高度 ──
    let mut cmd1 = Command::new(&chrome);
    cmd1.env("HOME", "/tmp/lianbot-chrome");
    for a in &base_args { cmd1.arg(a); }
    cmd1.args(["--virtual-time-budget=3000", "--dump-dom"]);
    cmd1.arg(format!("file://{html_path}"));

    let dom_output = cmd1.output().context("Chrome dump-dom 失败")?;
    let dom_str = String::from_utf8_lossy(&dom_output.stdout);
    let height = extract_title_height(&dom_str).unwrap_or(4000);
    let height = height.clamp(600, 20000);
    debug!("测量内容高度: {height}px");

    // ── 第二步：用精确高度截图 ──
    let mut cmd2 = Command::new(&chrome);
    cmd2.env("HOME", "/tmp/lianbot-chrome");
    for a in &base_args { cmd2.arg(a); }
    cmd2.args([
        "--hide-scrollbars",
        "--run-all-compositor-stages-before-draw",
        "--virtual-time-budget=5000",
    ]);
    cmd2.arg(format!("--screenshot={img_path}"));
    cmd2.arg(format!("--window-size=1200,{height}"));
    cmd2.arg(format!("file://{html_path}"));

    let output = cmd2.output().context("Chrome 截图失败")?;

    // 清理 HTML 临时文件
    let _ = std::fs::remove_file(&html_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Chrome 截图失败 (exit={}): {stderr}", output.status);
    }

    let img_data = std::fs::read(&img_path).context("读取截图文件失败")?;
    let _ = std::fs::remove_file(&img_path);

    let size_kb = img_data.len() / 1024;
    debug!("截图完成: {size_kb}KB PNG ({height}px 高)");

    Ok(B64.encode(&img_data))
}

/// 从 dump-dom 输出中提取 `<title>高度数字</title>`
fn extract_title_height(dom: &str) -> Option<u32> {
    let start = dom.find("<title>")? + 7;
    let end = dom[start..].find("</title>")? + start;
    dom[start..end].trim().parse().ok()
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
