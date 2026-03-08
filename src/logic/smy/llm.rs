use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};

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
r#"你是一个群聊信息总结助手。请分析以下群聊记录，提取出讨论最热烈、信息量最大的主要话题，每个话题必须聚焦于一个具体的讨论事件或子主题（例如：关于某部电影的讨论、一次线下聚会策划、一个技术问题解答）。如果同一主题下有多轮对话，可合并为一个话题。

要求：
- 话题数量：最多 8 个（如果超过 8 个，优先选择参与人数多、内容有深度的话题；如果少于 8 个，按实际情况返回）。
- 每个话题包含：
  1. 话题名称（简明扼要，能概括核心）
  2. 主要参与者（最多 5 人，按发言重要性排序）
  3. 详细描述（必须包含以下要素）：
     - 谁发起了该话题（@名字）
     - 哪些人参与了关键讨论（@名字）
     - 讨论了什么具体内容（关键信息、分歧点、笑点等）
     - 是否有结论或后续行动（如果有，请说明）
   - 描述中提及群友时，**必须使用 @名字 格式**（例如：@小雪 提出了周末聚餐的想法）。
   - 描述要具体，让只看总结的人也能了解讨论的来龙去脉。

群聊记录：
{messages_text}

请严格返回如下 JSON 数组（纯 JSON，无任何 markdown 标记）：
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
r#"你是一个有趣的群聊分析师。请根据以下群聊记录，为**最活跃、最有特色的群友**（发言次数多、能带动话题、金句频出等）生成专属称号，人数最多 6 人。

对于每个群友，请提供：
1. 群友昵称
2. 专属称号（4-8 字，有趣、贴切，体现其特点，如"深夜哲学家"、"表情包大师"）
3. MBTI 类型（根据发言风格娱乐性推测，如 E/I、N/S、T/F、J/P，若无法推测可写 "未知"）
4. 发言习惯（用生动有趣的语言总结，例如："喜欢在凌晨三点发表人生感悟，句句都像诗"、"每句话带三个表情包，自带弹幕效果"）
5. 理由（一句话解释为什么给这个称号，如：@小雪 经常在深夜分享人生感悟，引发群友思考）

同时，请挑选最多 5 条"群圣经"（群聊中令人印象深刻、有趣或富含哲理的发言）：
1. 原文内容（尽量保持原句，包括表情符号）
2. 发送者昵称
3. 入选理由（为什么这句话值得被记住，例如："完美概括了群友对加班的集体怨念"）

注意：
- 所有称号、理由必须积极正面，避免使用贬低或敏感词汇。
- 如果群聊记录不足，可减少条目，不要强行编造。

群聊记录：
{messages_text}

请严格返回如下 JSON（纯 JSON，无 markdown 标记）：
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
r#"你是一个善于解读人际关系的聚会侦探小将。请分析以下群聊记录中群友之间的互动模式，挖出 2-6 组最有知趣的关系（两人组合或多人团伙）。

要求：
- 每组关系需有明确成员名单（type 为 "duo" 或 "group"）。
- label 是 4-8 字的趣味小标签，可带点调皮，但要贴切（例如："摸鱼二人转"、"技术辩论组"）。
- vibe 是一句话点评关系的精髓，要生动、有依据，基于群聊中的实际互动模式推断（如：一个负责抛梗，一个负责接梗，默契十足）。
- evidence 引用一条具体的聊天内容作为佐证，必须是原文中的原句，保持完整，不加删改。如果原句包含表情或特殊符号，尽量保留。引用时请加双引号，例如：evidence: "@张三: 今晚通宵写代码吗？ @李四: 不了，我要睡觉，明天还要早起搬砖。"
- 如果记录不够充分，就少归纳几组，不要强行凑数。
- 标签和 vibe 需积极健康，不得含有低俗或人身攻击意味。

群聊记录：
{messages_text}

请严格返回如下 JSON 数组（纯 JSON，无 markdown 标记）：
[
  {{
    "type": "duo",
    "members": ["成员1", "成员2"],
    "label": "跑路搞码组合",
    "vibe": "一个冲、一个稳，互补型法宝",
    "evidence": "@张三: 今晚通宵写代码吗？ @李四: 不了，我要睡觉，明天还要早起搬砖。"
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
    info!("[llm] 开始分析: {} 条消息, {}KB 文本", messages.len(), formatted.len() / 1024);

    // 如果消息文本太短，跳过 LLM
    if formatted.len() < 50 {
        info!("[llm] 文本过短 ({}B), 跳过", formatted.len());
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

    let start = std::time::Instant::now();
    let (topics_res, titles_res, rel_res) = tokio::join!(
        call_llm(&config1, &topics_prompt),
        call_llm(&config2, &titles_prompt),
        call_llm(&config3, &relationships_prompt),
    );
    info!("[llm] 三路并行完成, 耗时 {:.1}s", start.elapsed().as_secs_f64());

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
    info!(
        "[llm] 结果: {} 话题, {} 称号, {} 金句, {} 关系",
        result.topics.len(),
        result.user_titles.len(),
        result.golden_quotes.len(),
        result.relationships.len(),
    );
    Ok(result)
}
