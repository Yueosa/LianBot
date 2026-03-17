// ── runtime::llm::gemini ───────────────────────────────────────────────────────
//
// Gemini API 客户端（多模态支持）。
//
// 职责：
//   - 调用 Google Gemini API 进行图片识别
//   - 支持多模态输入（文本 + 图片）
//   - 处理 Gemini 特有的消息格式
//
// 使用场景：
//   - vision 命令：识别用户发送的图片
//   - 其他需要多模态 AI 的场景
//
// 注意：
//   - Gemini 的消息格式与 OpenAI 不同
//   - 免费额度：1500 次/天，100 万 tokens/分钟
//   - 主要限制是请求次数，不是 token 数量

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ── 配置 ──────────────────────────────────────────────────────────────────────

/// runtime.toml `[gemini]` 段。
#[derive(Debug, Deserialize, Clone)]
pub struct GeminiConfig {
    /// Gemini API Key
    /// 获取地址：https://makersuite.google.com/app/apikey
    #[serde(default = "GeminiConfig::default_api_key")]
    pub api_key: String,

    /// 模型名称，默认 gemini-2.5-flash（最新的多模态模型）
    #[serde(default = "GeminiConfig::default_model")]
    pub model: String,

    /// 请求超时（秒），默认 60s
    #[serde(default = "GeminiConfig::default_timeout")]
    pub timeout_secs: u64,

    /// 图片识别提示词（可选，默认使用内置提示词）
    pub vision_prompt: Option<String>,
}

impl GeminiConfig {
    fn default_api_key() -> String { String::new() }
    fn default_model() -> String { "gemini-2.5-flash".to_string() }
    fn default_timeout() -> u64 { 60 }
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: Self::default_api_key(),
            model: Self::default_model(),
            timeout_secs: Self::default_timeout(),
            vision_prompt: None,
        }
    }
}

// ── 默认提示词 ────────────────────────────────────────────────────────────────

const DEFAULT_VISION_PROMPT: &str = "\
请详细描述这张图片的内容。重点说明：
- 图片的主要内容和主体
- 图片中的文字（如果有，请完整引用）
- 如果是表情包或梗图，请说明其含义、情绪或梗的背景
- 图片中值得注意的细节

请用自然的语言描述，不需要分点列举。";

// ── Gemini 请求/响应结构 ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    InlineData { inline_data: InlineData },
}

#[derive(Debug, Serialize)]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<Candidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Option<CandidateContent>,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    parts: Option<Vec<CandidatePart>>,
}

#[derive(Debug, Deserialize)]
struct CandidatePart {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
    #[serde(rename = "totalTokenCount")]
    total_token_count: Option<u32>,
}

// ── Gemini 客户端 ─────────────────────────────────────────────────────────────

/// Gemini API 客户端，持有连接配置。
///
/// 使用 runtime::http 提供的全局 HTTP 客户端，共享连接池。
pub struct GeminiClient {
    config: GeminiConfig,
    timeout: Duration,
}

impl GeminiClient {
    pub fn new(config: GeminiConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs);
        Self { config, timeout }
    }

    /// 分析图片内容
    ///
    /// # 参数
    /// - `image_b64`: base64 编码的图片数据
    /// - `prompt`: 可选的自定义提示词，None 则使用默认提示词或配置中的提示词
    ///
    /// # 返回
    /// - 图片描述文本
    ///
    /// # 错误
    /// - API 调用失败
    /// - 响应解析失败
    /// - 无有效候选结果
    ///
    /// # 示例
    /// ```rust
    /// let gemini = runtime::llm::gemini().unwrap();
    /// let description = gemini.analyze_image(&image_b64, None).await?;
    /// ```
    pub async fn analyze_image(
        &self,
        image_b64: &str,
        prompt: Option<&str>,
    ) -> Result<String> {
        // 确定使用的提示词：传入 > 配置 > 默认
        let prompt = prompt
            .or(self.config.vision_prompt.as_deref())
            .unwrap_or(DEFAULT_VISION_PROMPT);

        // 构造请求
        let request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![
                    GeminiPart::Text {
                        text: prompt.to_string(),
                    },
                    GeminiPart::InlineData {
                        inline_data: InlineData {
                            mime_type: "image/jpeg".to_string(),
                            data: image_b64.to_string(),
                        },
                    },
                ],
            }],
        };

        // 构造 URL
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.config.model, self.config.api_key
        );

        // 发送请求
        #[cfg(feature = "runtime-http")]
        let client = crate::runtime::http::client();
        #[cfg(not(feature = "runtime-http"))]
        let client = {
            static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
            CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .timeout(self.timeout)
                    .build()
                    .expect("构建 Gemini HTTP 客户端失败")
            })
        };

        let resp = client
            .post(&url)
            .timeout(self.timeout)
            .json(&request)
            .send()
            .await
            .context("Gemini API 请求失败")?;

        let status = resp.status();
        if !status.is_success() {
            let error_text = resp.text().await.unwrap_or_else(|_| "无法读取错误信息".to_string());
            anyhow::bail!("Gemini API 返回错误 {}: {}", status, error_text);
        }

        let data: GeminiResponse = resp.json().await
            .context("Gemini API 响应解析失败")?;

        // 提取响应文本
        let text = data.candidates
            .and_then(|candidates| candidates.into_iter().next())
            .and_then(|candidate| candidate.content)
            .and_then(|content| content.parts)
            .and_then(|parts| parts.into_iter().next())
            .and_then(|part| part.text)
            .context("Gemini API 响应中没有有效的文本内容")?;

        Ok(text)
    }

    /// 获取模型名称
    pub fn model(&self) -> &str {
        &self.config.model
    }
}

// ── 全局客户端 ────────────────────────────────────────────────────────────────

static GEMINI_CLIENT: OnceLock<GeminiClient> = OnceLock::new();

/// 初始化全局 Gemini 客户端（在 runtime::llm::init() 中调用）
///
/// 配置缺失或 API Key 为空时不初始化，后续 `gemini()` 返回 None。
pub fn init() {
    let cfg: Option<GeminiConfig> = crate::runtime::config::global()
        .section_opt("gemini");

    if let Some(cfg) = cfg {
        // 只有 API Key 不为空时才初始化
        if !cfg.api_key.is_empty() {
            GEMINI_CLIENT.get_or_init(|| GeminiClient::new(cfg));
        }
    }
}

/// 获取全局 Gemini 客户端（未配置时返回 None）
pub fn get() -> Option<&'static GeminiClient> {
    GEMINI_CLIENT.get()
}
