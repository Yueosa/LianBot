use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::{debug, warn};

const MIN_SCREENSHOT_HEIGHT: u32 = 600;
const MAX_SCREENSHOT_HEIGHT: u32 = 20_000;
const HEIGHT_SAFETY_PADDING: u32 = 40;

/// 单步 Chrome 操作的超时时间。
const CHROME_STEP_TIMEOUT: Duration = Duration::from_secs(30);

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

    let tmp_dir = tempfile::tempdir().context("创建截图临时目录失败")?;
    let tmp_path = tmp_dir.path();

    let html_path = tmp_path.join("page.html");
    let img_path  = tmp_path.join("shot.png");
    let user_data = tmp_path.join("chrome-profile");
    std::fs::create_dir_all(&user_data).context("创建 Chrome profile 目录失败")?;

    let user_data_arg = format!("--user-data-dir={}", user_data.display());
    let crash_dir_arg = format!("--crash-dumps-dir={}", tmp_path.join("crashes").display());

    // 注入 JS：将实际内容高度写入 <title>，供 dump-dom 步骤提取
    let patched_html = html.replace(
        "</body>",
        "<script>(function(){const de=document.documentElement;const b=document.body;\
         const scrollH=Math.max(de?de.scrollHeight:0,de?de.offsetHeight:0,de?de.clientHeight:0,\
         b?b.scrollHeight:0,b?b.offsetHeight:0,b?b.clientHeight:0);\
         let markerBottom=0;if(b){const marker=document.createElement('div');\
         marker.style.cssText='display:block;height:1px;width:1px;';\
         b.appendChild(marker);\
         markerBottom=marker.getBoundingClientRect().bottom+(window.scrollY||window.pageYOffset||0);\
         const bodyStyle=window.getComputedStyle(b);\
         markerBottom+=parseFloat(bodyStyle.paddingBottom)||0;}\
         const h=Math.ceil(Math.max(scrollH,markerBottom));\
         document.title=String(h);})();</script></body>",
    );
    std::fs::write(&html_path, &patched_html).context("写入临时 HTML 失败")?;

    let chrome = find_chrome()?;
    debug!("使用 Chrome: {chrome}");

    let base_args: Vec<&str> = vec![
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--disable-breakpad",
    ];

    // ── 第一步：dump-dom 测量内容高度 ──
    // 视口高度用 2000px（scrollHeight 不受限于视口大小，无需更大）
    let mut cmd1 = Command::new(&chrome);
    cmd1.env("HOME", tmp_path);
    cmd1.arg(&user_data_arg).arg(&crash_dir_arg);
    for a in &base_args { cmd1.arg(a); }
    cmd1.args([
        "--hide-scrollbars",
        "--virtual-time-budget=3000",
        "--dump-dom",
    ]);
    cmd1.arg(format!("--window-size={width},2000"));
    cmd1.arg(format!("file://{}", html_path.display()));

    let estimated = estimate_content_height(html);
    let measured_height = match run_with_timeout(&mut cmd1, "dump-dom") {
        Ok(dom_output) => {
            let dom_str = String::from_utf8_lossy(&dom_output.stdout);
            match extract_title_height(&dom_str) {
                Some(h) => {
                    debug!("dump-dom 测量成功: {h}px");
                    h
                }
                None => {
                    let dom_len = dom_str.len();
                    let title_snippet = dom_str.find("<title>").map(|s| {
                        let end = dom_str.len().min(s + 80);
                        dom_str[s..end].to_string()
                    });
                    warn!(
                        "dump-dom 高度提取失败 (dom={dom_len}B, title={title_snippet:?})，\
                         使用估算 {estimated}px"
                    );
                    estimated
                }
            }
        }
        Err(e) => {
            warn!("dump-dom 执行失败: {e:#}，使用估算 {estimated}px");
            estimated
        }
    };

    let target_height = measured_height.saturating_add(HEIGHT_SAFETY_PADDING);
    let height = target_height.clamp(MIN_SCREENSHOT_HEIGHT, MAX_SCREENSHOT_HEIGHT);
    debug!(
        "截图高度: measured={measured_height}px, estimated={estimated}px, final={height}px"
    );

    // ── 第二步：用精确高度截图 ──
    let mut cmd2 = Command::new(&chrome);
    cmd2.env("HOME", tmp_path);
    cmd2.arg(&user_data_arg).arg(&crash_dir_arg);
    for a in &base_args { cmd2.arg(a); }
    cmd2.args([
        "--hide-scrollbars",
        "--run-all-compositor-stages-before-draw",
        "--virtual-time-budget=5000",
    ]);
    cmd2.arg(format!("--screenshot={}", img_path.display()));
    cmd2.arg(format!("--window-size={width},{height}"));
    cmd2.arg(format!("file://{}", html_path.display()));

    let output = run_with_timeout(&mut cmd2, "截图")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Chrome 截图失败 (exit={}): {stderr}", output.status);
    }

    let img_data = std::fs::read(&img_path).context("读取截图文件失败")?;

    let size_kb = img_data.len() / 1024;
    debug!("截图完成: {size_kb}KB PNG ({width}x{height}), measured={measured_height}px");
    Ok(B64.encode(&img_data))
}

// ── 基于 HTML 内容的高度估算 ────────────────────────────────────────────────

/// 根据 HTML 中实际生成的 section 数量和内容量估算页面高度（px）。
/// 作为 dump-dom 测量失败时的回退方案，误差控制在 ±10%。
fn estimate_content_height(html: &str) -> u32 {
    // 固定部分：header(~150) + body padding(60) + footer(~80)
    let mut h: u32 = 300;

    // 基础统计：stats-grid 4 卡片
    if html.contains("stats-grid") { h += 160; }

    // 亮点一览：活跃时段 + 排行榜双栏
    if html.contains("highlights-grid") { h += 320; }

    // 24h 活跃度分布图（24 条 bar）
    if html.contains("chart-container") { h += 680; }

    // 热门话题（2 列 grid，每行 ~250px）
    let topics = html.matches("topic-item").count() as u32;
    if topics > 0 { h += 70 + ((topics + 1) / 2) * 260; }

    // 群友称号（2 列 grid，每行 ~200px）
    let titles = html.matches("user-card\"").count() as u32;
    if titles > 0 { h += 70 + ((titles + 1) / 2) * 210; }

    // 群圣经（单列，每条 ~140px）
    let quotes = html.matches("quote-item").count() as u32;
    if quotes > 0 { h += 70 + quotes * 150; }

    // 群友关系速写（2 列 grid，每行 ~230px）
    let rels = html.matches("rel-card\"").count() as u32;
    if rels > 0 { h += 70 + ((rels + 1) / 2) * 240; }

    h
}

/// 启动 Chrome 子进程并等待，超时则强制 kill。
fn run_with_timeout(
    cmd: &mut std::process::Command,
    label: &str,
) -> Result<std::process::Output> {
    let mut child = cmd.spawn().with_context(|| format!("Chrome {label} 启动失败"))?;

    let deadline = std::time::Instant::now() + CHROME_STEP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return child.wait_with_output()
                .with_context(|| format!("Chrome {label} 读取输出失败")),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // 回收僵尸进程
                    bail!("Chrome {label} 超时 ({}s)，已强制终止", CHROME_STEP_TIMEOUT.as_secs());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => bail!("Chrome {label} 等待失败: {e}"),
        }
    }
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
