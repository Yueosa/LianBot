pub mod fetcher;
pub mod statistics;
#[cfg(feature = "runtime-llm")]
pub mod llm;
pub mod renderer;
pub mod screenshot;

use anyhow::Result;
use serde::Deserialize;
use tracing::{debug, warn};

use self::fetcher::ChatMessage;

#[cfg(feature = "runtime-llm")]
pub use self::llm::{LlmResult, Topic, UserTitle, Quote, Relationship};

/// smy 插件配置，从 `logic.toml` 的 `[smy]` 段加载。
/// 所有字段均有默认值，`logic.toml` 不存在时也可正常运行。
///
/// LLM 配置已迁至 `runtime.toml [llm]`，通过 `runtime::llm::get()` 全局访问。
#[derive(Debug, Deserialize)]
pub struct SmyPluginConfig {
    /// 截图宽度（像素）
    #[serde(default = "SmyPluginConfig::default_screenshot_width")]
    pub screenshot_width: u32,
}

impl SmyPluginConfig {
    fn default_screenshot_width() -> u32 { 1200 }
}

impl Default for SmyPluginConfig {
    fn default() -> Self {
        Self {
            screenshot_width: SmyPluginConfig::default_screenshot_width(),
        }
    }
}

// ── LLM 结果占位符（无 runtime-llm 时使用） ──────────────────────────────────

#[cfg(not(feature = "runtime-llm"))]
#[derive(Debug, Clone, Default)]
pub struct LlmResult {
    pub topics: Vec<Topic>,
    pub user_titles: Vec<UserTitle>,
    pub golden_quotes: Vec<Quote>,
    pub relationships: Vec<Relationship>,
}

#[cfg(not(feature = "runtime-llm"))]
#[derive(Debug, Clone)]
pub struct Topic {
    pub topic: String,
    pub contributors: Vec<String>,
    pub detail: String,
}

#[cfg(not(feature = "runtime-llm"))]
#[derive(Debug, Clone)]
pub struct UserTitle {
    pub name: String,
    pub title: String,
    pub mbti: String,
    pub habit: String,
    pub reason: String,
}

#[cfg(not(feature = "runtime-llm"))]
#[derive(Debug, Clone)]
pub struct Quote {
    pub content: String,
    pub sender: String,
    pub reason: String,
}

#[cfg(not(feature = "runtime-llm"))]
#[derive(Debug, Clone)]
pub struct Relationship {
    pub rel_type: String,
    pub members: Vec<String>,
    pub label: String,
    pub vibe: String,
    pub evidence: Vec<String>,
}

// ── 公共管道 ──────────────────────────────────────────────────────────────────

/// smy 核心管道：统计 → 可选 LLM → 渲染 → 截图，返回 base64 PNG。
///
/// `with_ai` 为 true 时尝试获取全局 LlmClient 进行 AI 分析。
/// 调用方（命令 / 定时任务）负责消息拉取和图片发送，
/// 本函数只做纯计算 + 截图，不涉及 API 交互。
pub async fn generate_report(
    messages: &[ChatMessage],
    with_ai: bool,
    group_label: &str,
    screenshot_width: u32,
) -> Result<String> {
    // ── 统计分析 ──────────────────────────────────────────────────────────────
    let stats = statistics::analyze(messages);

    // ── LLM 分析（可选） ──────────────────────────────────────────────────────
    #[cfg(feature = "runtime-llm")]
    let llm_result = if with_ai {
        if let Some(client) = crate::runtime::llm::get() {
            debug!("[smy] 请求 LLM 分析...");
            match llm::analyze(messages, client).await {
                Ok(r) => {
                    debug!("[smy] LLM 分析完成");
                    r
                }
                Err(e) => {
                    warn!("[smy] LLM 分析失败，使用空结果: {e:#}");
                    llm::LlmResult::default()
                }
            }
        } else {
            warn!("[smy] with_ai=true 但 LLM 未配置，跳过");
            llm::LlmResult::default()
        }
    } else {
        llm::LlmResult::default()
    };

    #[cfg(not(feature = "runtime-llm"))]
    let llm_result = {
        if with_ai {
            warn!("[smy] with_ai=true 但 runtime-llm 未编译，跳过");
        }
        LlmResult::default()
    };

    // ── 渲染 HTML ─────────────────────────────────────────────────────────────
    let html = renderer::render(&stats, &llm_result, group_label, messages);
    debug!("[smy] 渲染完成: HTML {}KB", html.len() / 1024);

    // ── 截图 ──────────────────────────────────────────────────────────────────
    let hint = screenshot::estimate_height(&llm_result);
    let base64_img = screenshot::capture(&html, screenshot_width, hint).await?;
    debug!("[smy] 截图完成: {}KB", base64_img.len() / 1024);

    Ok(base64_img)
}
