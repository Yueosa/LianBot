pub mod client;

use std::sync::OnceLock;

use serde::Deserialize;

pub use client::LlmClient;

static LLM_CLIENT: OnceLock<LlmClient> = OnceLock::new();

/// 初始化全局 LLM 客户端（boot.rs 调用一次）。
/// 配置缺失时不初始化，后续 `get()` 返回 None。
pub fn init() {
    let cfg: Option<LlmConfig> = crate::runtime::config::global()
        .section_opt("llm");

    if let Some(cfg) = cfg {
        LLM_CLIENT.get_or_init(|| LlmClient::new(cfg));
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
    #[serde(default = "LlmConfig::default_api_key")]
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
    fn default_api_key() -> String { String::new() }
    fn default_model() -> String { "deepseek-chat".to_string() }
    fn default_timeout() -> u64 { 120 }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_url: Self::default_url(),
            api_key: Self::default_api_key(),
            model: Self::default_model(),
            timeout_secs: Self::default_timeout(),
        }
    }
}
