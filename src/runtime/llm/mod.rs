pub mod client;

use std::sync::OnceLock;

use serde::Deserialize;
use tracing::info;

pub use client::LlmClient;

static LLM_CLIENT: OnceLock<LlmClient> = OnceLock::new();

/// 初始化全局 LLM 客户端（boot.rs 调用一次）。
/// 配置缺失时不初始化，后续 `get()` 返回 None。
pub fn init() {
    let cfg: Option<LlmConfig> = crate::runtime::config::global()
        .section_opt("llm");

    if let Some(cfg) = cfg {
        info!("[llm] 已配置: {} / {}", cfg.api_url, cfg.model);
        LLM_CLIENT.get_or_init(|| LlmClient::new(cfg));
    } else {
        info!("[llm] 未配置 [llm] 段，LLM 功能不可用");
    }
}

/// 获取全局 LLM 客户端（未配置时返回 None）。
pub fn get() -> Option<&'static LlmClient> {
    LLM_CLIENT.get()
}

// ── LLM 配置 ──────────────────────────────────────────────────────────────────

/// runtime.toml `[llm]` 段。
/// 提供 OpenAI 兼容 API 的连接信息，供全局 LlmClient 使用。
#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    /// OpenAI 兼容 API 地址
    #[serde(default = "LlmConfig::default_url")]
    pub api_url: String,
    /// API Key
    pub api_key: String,
    /// 默认模型名称
    #[serde(default = "LlmConfig::default_model")]
    pub model: String,
    /// 请求超时（秒），默认 120
    #[serde(default = "LlmConfig::default_timeout")]
    pub timeout_secs: u64,
}

impl LlmConfig {
    fn default_url() -> String { "https://api.deepseek.com/v1".to_string() }
    fn default_model() -> String { "deepseek-chat".to_string() }
    fn default_timeout() -> u64 { 120 }
}
