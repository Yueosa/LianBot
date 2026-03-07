use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::warn;

use crate::logic::smy::LlmConfig;
use super::fetcher::{ChatMessage, format_for_llm};

// ── LLM 分析结果 ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct LlmResult {
    pub topics: Vec<Topic>,
    pub user_titles: Vec<UserTitle>,
    pub golden_quotes: Vec<Quote>,
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Relationship {
    /// "duo"（两人）或 "group"（多人团伙）
    #[serde(rename = "type")]
    pub rel_type: String,
    pub members: Vec<String>,
    /// 4-6 字趣味短标签
    pub label: String,
    /// 一句话关系概括
    pub vibe: String,
    /// 具体聊天作为佐证
    pub evidence: String,
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
- 在描述中提到群友名字时，必须使用 @名字 格式标记（例如：@小雪 发起了讨论）
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

同时，请挑选最多5条"群圣经"（印象深刻、有趣或有哲理的发言）：
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

fn build_relationships_prompt(messages_text: &str) -> String {
    format!(
r#"你是一个善于解读人际关系的聚会侦探小将，对群聊充满好奇心，既专业又有趣。请分析以下群聊记录中群友之间的互动模式，挖出 3-5 组最有知趣的关系。

要求：
- 可以是两人关系（duo），也可以是多人团伙（group）
- label 是 4-6 字的趣味小标签，可以有点小小的调皮味道
- vibe 是一句话点评关系的精髓，要生动、推断得有根据、读起来不要屈履平平
- evidence 引用一条具体聊天作为佐证，尽量是原文里足够有趣的句子
- 如果记录不够充分就少归纳几组，不要勇强凑数

群聊记录：
{messages_text}

请严格返回如下 JSON 数组（纯 JSON，无 markdown 标记）：
[
  {{
    "type": "duo",
    "members": ["成员1", "成员2"],
    "label": "跑路搞码组合",
    "vibe": "一个冲、一个稳，互补型法宝",
    "evidence": "引用的具体聊天内容"
  }}
]"#
    )
}

// ── LLM 请求 ──────────────────────────────────────────────────────────────────

async fn call_llm(config: &LlmConfig, prompt: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
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

    // 三次 LLM 调用并行执行，大幅减少等待时间
    let topics_prompt        = build_topics_prompt(truncated);
    let titles_prompt        = build_titles_prompt(truncated);
    let relationships_prompt = build_relationships_prompt(truncated);
    let config1 = config.clone();
    let config2 = config.clone();
    let config3 = config.clone();

    let (topics_res, titles_res, rel_res) = tokio::join!(
        call_llm(&config1, &topics_prompt),
        call_llm(&config2, &titles_prompt),
        call_llm(&config3, &relationships_prompt),
    );

    // 解析话题结果
    match topics_res {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            match serde_json::from_str::<Vec<Topic>>(cleaned) {
                Ok(topics) => result.topics = topics,
                Err(e) => warn!("话题 JSON 解析失败: {e}\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 话题分析失败: {e}"),
    }

    // 解析称号+金句结果
    match titles_res {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
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
    // 解析关系报告结果
    match rel_res {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            match serde_json::from_str::<Vec<Relationship>>(cleaned) {
                Ok(rels) => result.relationships = rels,
                Err(e) => warn!("关系报告 JSON 解析失败: {e}\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 关系报告失败: {e}"),
    }
    Ok(result)
}
