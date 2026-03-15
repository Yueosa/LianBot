use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tracing::{debug, warn};

#[cfg(feature = "runtime-llm")]
use super::llm::LlmResult;

#[cfg(not(feature = "runtime-llm"))]
use super::LlmResult;

const MIN_SCREENSHOT_HEIGHT: u32 = 600;
const MAX_SCREENSHOT_HEIGHT: u32 = 20_000;

/// Chrome 单步操作的超时时间。
const CHROME_STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// dump-dom 测量用的视口高度（足够小以节省内存，scrollHeight 不受视口限制）
const MEASURE_VIEWPORT_HEIGHT: u32 = 800;

// ── HTML → PNG → base64 ─────────────────────────────────────────────────────

/// 将 HTML 字符串渲染为截图，返回 PNG base64 编码。
///
/// `hint_height` 是基于内容结构体的高度估算值，在 Chrome dump-dom 测量失败时用作回退。
pub async fn capture(html: &str, width: u32, hint_height: u32) -> Result<String> {
    let html_owned = html.to_string();
    tokio::task::spawn_blocking(move || capture_sync(&html_owned, width, hint_height))
        .await
        .context("截图任务 panic")?
}

fn capture_sync(html: &str, width: u32, hint_height: u32) -> Result<String> {
    use std::process::{Command, Stdio};

    let tmp_dir = tempfile::tempdir().context("创建截图临时目录失败")?;
    let tmp_path = tmp_dir.path();

    let html_path = tmp_path.join("page.html");
    let img_path  = tmp_path.join("shot.png");
    let user_data = tmp_path.join("chrome-profile");
    std::fs::create_dir_all(&user_data).context("创建 Chrome profile 目录失败")?;

    let user_data_arg = format!("--user-data-dir={}", user_data.display());
    let crash_dir_arg = format!("--crash-dumps-dir={}", tmp_path.join("crashes").display());

    // 注入高度测量 JS：在 body 末尾追加 marker，将真实内容高度写入 <title>
    let patched_html = inject_measure_js(html);
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

    // ── 第一步：dump-dom 测量真实内容高度 ──
    // 视口仅 800px（scrollHeight 不受视口限制），内存占用极低
    let mut cmd1 = Command::new(&chrome);
    cmd1.env("HOME", tmp_path);
    cmd1.arg(&user_data_arg).arg(&crash_dir_arg);
    for a in &base_args { cmd1.arg(a); }
    cmd1.args([
        "--hide-scrollbars",
        "--virtual-time-budget=5000",
        "--dump-dom",
    ]);
    cmd1.arg(format!("--window-size={width},{MEASURE_VIEWPORT_HEIGHT}"));
    cmd1.arg(format!("file://{}", html_path.display()));
    cmd1.stdout(Stdio::piped()).stderr(Stdio::piped());

    let content_height = match run_with_timeout(&mut cmd1, "dump-dom") {
        Ok(output) => {
            let dom = String::from_utf8_lossy(&output.stdout);
            match extract_title_height(&dom) {
                Some(h) if h >= 200 && h <= 15000 => {
                    debug!("dump-dom 测量成功: {h}px");
                    h
                }
                Some(h) => {
                    warn!("dump-dom 返回异常高度 {h}px，使用估算 {hint_height}px");
                    hint_height
                }
                None => {
                    warn!("dump-dom 高度提取失败，使用估算 {hint_height}px");
                    hint_height
                }
            }
        }
        Err(e) => {
            warn!("dump-dom 执行失败: {e:#}，使用估算 {hint_height}px");
            hint_height
        }
    };

    // dump-dom 测量值加 30px body bottom padding 余量；回退估算值加 80px（信任度较低）
    let padding = if content_height == hint_height { 80 } else { 30 };
    let height = content_height
        .saturating_add(padding)
        .clamp(MIN_SCREENSHOT_HEIGHT, MAX_SCREENSHOT_HEIGHT);
    debug!("截图高度: measured/hint={content_height}px, padding={padding}px, final={height}px");

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
    debug!("截图完成: {size_kb}KB PNG ({width}x{height})");
    Ok(B64.encode(&img_data))
}

// ── JS 注入 & 高度提取 ──────────────────────────────────────────────────────

/// 在 `</body>` 前注入测量脚本：在 body 末尾追加 0px marker，
/// 取其 getBoundingClientRect().bottom + scrollY + body paddingBottom 作为真实内容高度，
/// 写入 `<title>`。不受 viewport 大小影响。
fn inject_measure_js(html: &str) -> String {
    html.replace(
        "</body>",
        r#"<script>(function(){
var b=document.body;
var m=document.createElement('div');m.style.cssText='height:0;clear:both;';
b.appendChild(m);
var r=m.getBoundingClientRect();
var scrollY=window.scrollY||window.pageYOffset||0;
var bottom=Math.ceil(r.bottom+scrollY);
var cs=window.getComputedStyle(b);
bottom+=Math.ceil(parseFloat(cs.paddingBottom)||0);
document.title=String(bottom);
})();</script></body>"#,
    )
}

/// 从 dump-dom 输出中提取 `<title>数字</title>`。
fn extract_title_height(dom: &str) -> Option<u32> {
    let start = dom.find("<title>")? + 7;
    let end = dom[start..].find("</title>")? + start;
    dom[start..end].trim().parse().ok()
}

// ── 基于结构体的高度估算（dump-dom 失败时的回退） ────────────────────────────

/// 根据 LLM 分析结果估算报告页面高度（px）。
///
/// 不做 HTML 字符串匹配——直接看有多少 topics / titles / quotes / relationships，
/// 连同固定区块和 section margin 一起计算。
///
/// 系数基于 Chrome 实测校准（1200px 宽度，Noto Sans SC 字体）。
pub fn estimate_height(llm: &LlmResult) -> u32 {
    // ── 固定区块（始终存在） ──
    // body padding top(30) + header(~170) + content padding top(40)
    // + footer(~107) + body padding bottom(30) = ~377, 取 380
    let mut h: u32 = 380;

    // 基础统计 stats-grid (~145px) + section margin(40px)
    h += 185;
    // 亮点一览 highlights-grid (~290px) + section margin(40px)
    h += 330;
    // 24h 活跃度图表 (~740px) + section margin(40px)
    h += 780;

    // ── AI 区块（按实际条目数计算） ──

    let n_topics = llm.topics.len() as u32;
    if n_topics > 0 {
        // section-title(~36px) + grid rows(每行 ~240px, 2列) + section margin(40px)
        h += 76 + ((n_topics + 1) / 2) * 240;
    }

    let n_titles = llm.user_titles.len() as u32;
    if n_titles > 0 {
        // section-title(~36px) + grid rows(每行 ~200px, 2列) + section margin(40px)
        h += 76 + ((n_titles + 1) / 2) * 200;
    }

    let n_quotes = llm.golden_quotes.len() as u32;
    if n_quotes > 0 {
        // section-title(~36px) + items(每条 ~130px) + section margin(40px)
        h += 76 + n_quotes * 130;
    }

    let n_rels = llm.relationships.len() as u32;
    if n_rels > 0 {
        // section-title(~36px) + grid rows(每行 ~220px, 2列)
        // 最后一个 section 无 margin-bottom（CSS last-child 规则）
        h += 36 + ((n_rels + 1) / 2) * 220;
    }

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
