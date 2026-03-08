pub mod fetcher;
pub mod statistics;
pub mod llm;
pub mod renderer;
pub mod screenshot;

use anyhow::Result;
use serde::Deserialize;
use tracing::{info, warn};

use self::fetcher::ChatMessage;

// ── LLM 配置 ──────────────────────────────────────────────────────────────────

/// logic.toml `[smy.llm]` 段。
#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    /// OpenAI 兼容 API 地址
    #[serde(default = "LlmConfig::default_url")]
    pub api_url: String,
    /// API Key
    pub api_key: String,
    /// 模型名称
    #[serde(default = "LlmConfig::default_model")]
    pub model: String,
}

impl LlmConfig {
    fn default_url() -> String { "https://api.deepseek.com/v1".to_string() }
    fn default_model() -> String { "deepseek-chat".to_string() }
}

/// smy 插件配置，从 `logic.toml` 的 `[smy]` 段加载。
/// 所有字段均有默认值，`logic.toml` 不存在时也可正常运行。
#[derive(Debug, Deserialize)]
pub struct SmyPluginConfig {
    /// 截图宽度（像素）
    #[serde(default = "SmyPluginConfig::default_screenshot_width")]
    pub screenshot_width: u32,
    /// LLM 配置（可选，缺少时 -a/--ai 报错提示未配置）
    pub llm: Option<LlmConfig>,
}

impl SmyPluginConfig {
    fn default_screenshot_width() -> u32 { 1200 }
}

impl Default for SmyPluginConfig {
    fn default() -> Self {
        Self {
            screenshot_width: SmyPluginConfig::default_screenshot_width(),
            llm: None,
        }
    }
}

// ── 公共管道 ──────────────────────────────────────────────────────────────────

/// smy 核心管道：统计 → 可选 LLM → 渲染 → 截图，返回 base64 PNG。
///
/// 调用方（命令 / 定时任务）负责消息拉取和图片发送，
/// 本函数只做纯计算 + 截图，不涉及 API 交互。
pub async fn generate_report(
    messages: &[ChatMessage],
    llm_config: Option<&LlmConfig>,
    group_label: &str,
    screenshot_width: u32,
) -> Result<String> {
    // ── 统计分析 ──────────────────────────────────────────────────────────────
    let stats = statistics::analyze(messages);

    // ── LLM 分析（可选） ──────────────────────────────────────────────────────
    let llm_result = if let Some(cfg) = llm_config {
        info!("[smy] 请求 LLM 分析...");
        match llm::analyze(messages, cfg).await {
            Ok(r) => {
                info!("[smy] LLM 分析完成");
                r
            }
            Err(e) => {
                warn!("[smy] LLM 分析失败，使用空结果: {e:#}");
                llm::LlmResult::default()
            }
        }
    } else {
        llm::LlmResult::default()
    };

    // ── 渲染 HTML ─────────────────────────────────────────────────────────────
    let html = renderer::render(&stats, &llm_result, group_label, messages);
    info!("[smy] 渲染完成: HTML {}KB", html.len() / 1024);

    // ── 截图 ──────────────────────────────────────────────────────────────────
    let base64_img = screenshot::capture(&html, screenshot_width).await?;
    info!("[smy] 截图完成: {}KB", base64_img.len() / 1024);

    Ok(base64_img)
}
