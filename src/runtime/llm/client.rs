use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use super::LlmConfig;

/// 全局 LLM 客户端，持有连接配置和 HTTP Client。
///
/// 职责：底层 HTTP 通信、鉴权、超时。
/// 业务逻辑（prompt 构造、response 解析）由调用方负责。
pub struct LlmClient {
    client: reqwest::Client,
    config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("构建 LLM HTTP 客户端失败");
        Self { client, config }
    }

    /// 模型名称（调用方可能需要写入 request body）
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// 发送 chat completion 请求，返回 assistant 回复的文本内容。
    ///
    /// `messages` 是 OpenAI 格式的消息数组，如 `[{"role":"user","content":"..."}]`。
    /// `temperature` 和 `max_tokens` 由调用方按场景指定。
    pub async fn chat(
        &self,
        messages: &[Value],
        temperature: f64,
        max_tokens: u32,
    ) -> Result<String> {
        let url = format!(
            "{}/chat/completions",
            self.config.api_url.trim_end_matches('/')
        );

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "response_format": { "type": "json_object" },
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("LLM 请求发送失败")?;

        let status = resp.status();
        let resp_body: Value = resp.json().await.context("LLM 响应解析失败")?;

        if !status.is_success() {
            anyhow::bail!("LLM API 返回 {status}: {resp_body}");
        }

        let content = resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }
}
