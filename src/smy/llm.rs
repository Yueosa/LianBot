use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::warn;

use crate::config::LlmConfig;
use super::fetcher::{ChatMessage, format_for_llm};

// ── LLM 分析结果 ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct LlmResult {
    pub topics: Vec<Topic>,
    pub user_titles: Vec<UserTitle>,
    pub golden_quotes: Vec<Quote>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Topic {
    pub topic: String,
    pub contributors: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserTitle {
    pub name: String,
    pub title: String,
    pub mbti: String,
    pub habit: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Quote {
    pub content: String,
    pub sender: String,
    pub reason: String,
}

// ── Prompt 构造 ───────────────────────────────────────────────────────────────

fn build_topics_prompt(messages_text: &str) -> String {
    format!(
r#"你是一个群聊信息总结助手。请分析以下群聊记录，提取出最多6个主要话题。

对于每个话题，请提供：
1. 话题名称（简明扼要）
2. 主要参与者（最多5人）
3. 详细描述（包含关键信息、前因后果和结论，让人只看总结就能了解讨论的有价值内容）

注意：
- 要具体说明是哪个群友做了什么、说了什么，而非宽泛概述
- 对于有价值的点，详细讲一两句，让读者只看总结就能获取关键信息
- 每条总结尽量讲清楚前因后果

群聊记录：
{messages_text}

请严格返回如下JSON数组（纯JSON，无markdown标记）：
[
  {{
    "topic": "话题名称",
    "contributors": ["用户1", "用户2"],
    "detail": "话题描述内容"
  }}
]"#
    )
}

fn build_titles_prompt(messages_text: &str) -> String {
    format!(
r#"你是一个有趣的群聊分析师。请根据以下群聊记录，为最活跃的群友（最多6人）生成专属称号。

对于每个群友，请提供：
1. 群友昵称
2. 专属称号（有趣、贴切，如"深夜哲学家"、"表情包大师"）
3. MBTI 类型（根据发言风格推测）
4. 发言习惯（用有趣的方式总结，如"喜欢在凌晨三点发表人生感悟"、"每句话都带表情包"、"总是用省略号结尾..."）
5. 理由（简短解释为什么给这个称号）

同时，请挑选最多4条"群圣经"（印象深刻、有趣或有哲理的发言）：
1. 原文内容
2. 发送者昵称
3. 入选理由（为什么这句话值得被记住）

群聊记录：
{messages_text}

请严格返回如下JSON（纯JSON，无markdown标记）：
{{
  "user_titles": [
    {{
      "name": "群友昵称",
      "title": "专属称号",
      "mbti": "XXXX",
      "habit": "发言习惯描述",
      "reason": "给出称号的理由"
    }}
  ],
  "golden_quotes": [
    {{
      "content": "原文",
      "sender": "发送者",
      "reason": "入选理由"
    }}
  ]
}}"#
    )
}

// ── LLM 请求 ──────────────────────────────────────────────────────────────────

async fn call_llm(config: &LlmConfig, prompt: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;

    let url = format!("{}/chat/completions", config.api_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            {
                "role": "user",
                "content": prompt,
            }
        ],
        "temperature": 0.7,
        "max_tokens": 4096,
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("LLM 请求发送失败")?;

    let status = resp.status();
    let resp_body: serde_json::Value = resp.json().await.context("LLM 响应解析失败")?;

    if !status.is_success() {
        anyhow::bail!("LLM API 返回 {status}: {resp_body}");
    }

    let content = resp_body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(content)
}

/// 清理 LLM 返回中可能包的 markdown 代码块标记
fn clean_json(raw: &str) -> &str {
    let s = raw.trim();
    let s = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}

// ── 分析入口 ──────────────────────────────────────────────────────────────────

pub async fn analyze(messages: &[ChatMessage], config: &LlmConfig) -> Result<LlmResult> {
    let formatted = format_for_llm(messages);

    // 如果消息文本太短，跳过 LLM
    if formatted.len() < 50 {
        return Ok(LlmResult::default());
    }

    // 截断保护: DeepSeek 128k 上下文, 保守限制在 ~80k 字符
    let truncated = if formatted.len() > 80_000 {
        &formatted[formatted.len() - 80_000..]
    } else {
        &formatted
    };

    let mut result = LlmResult::default();

    // 第一次调用：话题总结
    match call_llm(config, &build_topics_prompt(truncated)).await {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            match serde_json::from_str::<Vec<Topic>>(cleaned) {
                Ok(topics) => result.topics = topics,
                Err(e) => warn!("话题 JSON 解析失败: {e}\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 话题分析失败: {e}"),
    }

    // 第二次调用：称号 + 金句
    match call_llm(config, &build_titles_prompt(truncated)).await {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            // 解析外层 JSON 对象
            match serde_json::from_str::<serde_json::Value>(cleaned) {
                Ok(v) => {
                    if let Some(arr) = v.get("user_titles") {
                        match serde_json::from_value::<Vec<UserTitle>>(arr.clone()) {
                            Ok(titles) => result.user_titles = titles,
                            Err(e) => warn!("称号 JSON 解析失败: {e}"),
                        }
                    }
                    if let Some(arr) = v.get("golden_quotes") {
                        match serde_json::from_value::<Vec<Quote>>(arr.clone()) {
                            Ok(quotes) => result.golden_quotes = quotes,
                            Err(e) => warn!("金句 JSON 解析失败: {e}"),
                        }
                    }
                }
                Err(e) => warn!("称号/金句 JSON 解析失败: {e}\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 称号分析失败: {e}"),
    }

    Ok(result)
}
