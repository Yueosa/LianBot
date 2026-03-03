use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::debug;

const MEASURE_VIEWPORT_HEIGHT: u32 = 2000;
const MIN_SCREENSHOT_HEIGHT: u32 = 600;
const MAX_SCREENSHOT_HEIGHT: u32 = 20_000;
const HEIGHT_SAFETY_PADDING: u32 = 24;

// ── HTML → PNG → base64 ─────────────────────────────────────────────────────

/// 将 HTML 字符串渲染为截图，返回 PNG base64 编码。
/// 依赖系统安装的 Chrome / Chromium（通过 CLI `--screenshot` 模式）。
/// `width` 为截图宽度（像素），通常取 `SmyPluginConfig::screenshot_width`。
pub async fn capture(html: &str, width: u32) -> Result<String> {
    let html_owned = html.to_string();
    tokio::task::spawn_blocking(move || capture_sync(&html_owned, width))
        .await
        .context("截图任务 panic")?
}

fn capture_sync(html: &str, width: u32) -> Result<String> {
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
        "<script>(function(){const de=document.documentElement;const b=document.body;const scrollH=Math.max(de?de.scrollHeight:0,de?de.offsetHeight:0,de?de.clientHeight:0,b?b.scrollHeight:0,b?b.offsetHeight:0,b?b.clientHeight:0);let markerBottom=0;if(b){const marker=document.createElement('div');marker.style.cssText='display:block;height:1px;width:1px;';b.appendChild(marker);markerBottom=marker.getBoundingClientRect().bottom+(window.scrollY||window.pageYOffset||0);const bodyStyle=window.getComputedStyle(b);markerBottom+=parseFloat(bodyStyle.paddingBottom)||0;}const h=Math.ceil(Math.max(scrollH,markerBottom));document.title=String(h);})();</script></body>",
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
    cmd1.args([
        "--hide-scrollbars",
        "--virtual-time-budget=3000",
        "--dump-dom",
    ]);
    cmd1.arg(format!("--window-size={width},{MEASURE_VIEWPORT_HEIGHT}"));
    cmd1.arg(format!("file://{html_path}"));

    let dom_output = cmd1.output().context("Chrome dump-dom 失败")?;
    let dom_str = String::from_utf8_lossy(&dom_output.stdout);
    let measured_height = extract_title_height(&dom_str).unwrap_or(4000);
    let target_height = measured_height.saturating_add(HEIGHT_SAFETY_PADDING);
    let height = target_height.clamp(MIN_SCREENSHOT_HEIGHT, MAX_SCREENSHOT_HEIGHT);
    debug!(
        "测量内容高度: measured={}px, padded={}px, final={}px, width={}px",
        measured_height,
        target_height,
        height,
        width
    );

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
    cmd2.arg(format!("--window-size={width},{height}"));
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
    debug!(
        "截图完成: {size_kb}KB PNG ({}x{}), measured={}px",
        width,
        height,
        measured_height
    );

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_sync_smoke() {
        if find_chrome().is_err() {
            eprintln!("跳过截图测试：系统未安装 Chrome/Chromium");
            return;
        }

        let html = r#"<!DOCTYPE html><html><head><meta charset=\"UTF-8\"></head><body style=\"margin:0;padding:30px;background:#5BCEFA;\"><div style=\"width:1200px;margin:0 auto;background:#fff;border-radius:16px;padding:24px;\"><h1>smy smoke</h1><p>hello screenshot</p><div style=\"height:1200px;background:#f8fafc;\"></div><footer style=\"margin-top:12px;background:#5BCEFA;color:#fff;padding:12px;\">footer</footer></div></body></html>"#;

        let b64 = capture_sync(html, 1200).expect("capture_sync should succeed");
        assert!(!b64.is_empty(), "base64 output should not be empty");

        let bytes = B64.decode(b64).expect("base64 should decode");
        assert!(bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]), "output should be png");
        assert!(bytes.len() > 1024, "png bytes should have enough size");
    }
}
