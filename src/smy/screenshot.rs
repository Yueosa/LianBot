use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::debug;

// ── HTML → PNG → base64 ─────────────────────────────────────────────────────

/// 将 HTML 字符串渲染为截图，返回 JPEG base64 编码
pub async fn capture(html: &str) -> Result<String> {
    let html_owned = html.to_string();

    tokio::task::spawn_blocking(move || capture_sync(&html_owned))
        .await
        .context("截图任务 panic")?
}

fn capture_sync(html: &str) -> Result<String> {
    use headless_chrome::{Browser, LaunchOptions};
    use headless_chrome::protocol::cdp::Page;

    let options = LaunchOptions {
        headless: true,
        sandbox: false, // root 运行需要 --no-sandbox
        window_size: Some((1200, 800)),
        args: vec![
            std::ffi::OsStr::new("--disable-gpu"),
            std::ffi::OsStr::new("--no-sandbox"),
            std::ffi::OsStr::new("--disable-dev-shm-usage"),
            std::ffi::OsStr::new("--font-render-hinting=none"),
        ],
        ..LaunchOptions::default()
    };

    let browser = Browser::new(options).context("启动 headless chrome 失败")?;
    let tab = browser.new_tab().context("创建标签页失败")?;

    // 写入临时文件（data URL 对大 HTML 不稳定）
    let tmp_path = format!("/tmp/lianbot_smy_{}.html", std::process::id());
    std::fs::write(&tmp_path, html).context("写入临时 HTML 失败")?;

    let file_url = format!("file://{tmp_path}");
    tab.navigate_to(&file_url)
        .context("导航到 HTML 失败")?;
    tab.wait_until_navigated()
        .context("等待页面加载失败")?;

    // 等待 body 渲染完成
    tab.wait_for_element("body")
        .context("等待 body 元素失败")?;

    // 短暂等待确保 CSS 渲染完成
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 全页面截图 → JPEG（压缩体积，quality=85 保持清晰）
    let screenshot_data = tab
        .capture_screenshot(
            Page::CaptureScreenshotFormatOption::Jpeg,
            Some(85),
            None,
            true, // capture_beyond_viewport = full page
        )
        .context("截图失败")?;

    // 清理临时文件
    let _ = std::fs::remove_file(&tmp_path);

    let size_kb = screenshot_data.len() / 1024;
    debug!("截图完成: {size_kb}KB JPEG");

    Ok(B64.encode(&screenshot_data))
}
