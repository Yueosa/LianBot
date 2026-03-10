use anyhow::Result;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::runtime::llm::LlmClient;
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
    /// 具体聊天佐证（1-3 条）
    #[serde(deserialize_with = "deserialize_string_or_vec")]
    pub evidence: Vec<String>,
}

/// 兼容反序列化：接受 JSON string 或 string array
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrVec;
    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or array of strings")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Vec<String>, E> {
            Ok(vec![v.to_string()])
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Vec<String>, E> {
            Ok(vec![v])
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<String>, A::Error> {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                out.push(s);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_any(StringOrVec)
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
     - 如果讨论产生了明确结论或行动计划，简要说明；无明确结论时无需提及
   - 描述中提及群友时，**必须使用 @名字 格式**（例如：@小雪 提出了周末聚餐的想法）。
   - 描述要具体，让只看总结的人也能了解讨论的来龙去脉。

群聊记录：
{messages_text}

请严格返回如下 JSON 对象（纯 JSON，无任何 markdown 标记）：
{{
  "topics": [
    {{
      "topic": "话题名称",
      "contributors": ["用户1", "用户2"],
      "detail": "话题描述内容"
    }}
  ]
}}"#
    )
}

fn build_titles_prompt(messages_text: &str) -> String {
    format!(
r#"你是一个精通中文互联网文化的群聊分析鬼才。请根据以下群聊记录，为**最活跃、最有特色的群友**（发言次数多、能带动话题、金句频出等）生成专属称号，人数最多 6 人。

对于每个群友，请提供：
1. 群友昵称
2. 专属称号（4-8 字，要有梗、有网感，体现该群友最鲜明的特质。
   好例子："赛博嘴替"、"复读机本机"、"表情包战神"、"群里唯一清醒的人"、"午夜电台 DJ"、"话题终结者"、"已读不回重灾区"
   差例子：太正式或太万金油的称号，如"活跃达人"、"群聊之星"——这种不够有趣）
3. MBTI 类型（根据发言风格娱乐性推测，如 E/I、N/S、T/F、J/P，若无法推测可写 "未知"）
4. 发言习惯（用最生动损友式的语言总结，例如："喜欢在凌晨三点发表人生感悟，第二天自己都看不懂"、"每句话带三个表情包，关掉表情包约等于社恐"、"上来就是一个转发，生怕群里不够热闹"）
5. 理由（一句话解释为什么给这个称号——要具体到聊天内容，不要泛泛而谈）

同时，请挑选最多 5 条"群圣经"（群聊中令人印象深刻、有趣或富含哲理的发言）：
1. 原文内容（尽量保持原句，包括表情符号）
2. 发送者昵称
3. 入选理由（为什么这句话值得被记住——要写得让人想笑或想截图，例如："一句话说出了全群打工人的心声"、"在最不经意的时刻投下了一颗深水炸弹"）

注意：
- 玩梗可以，但不要人身攻击或涉及敏感话题。整体氛围应该是"损友式吐槽"而非恶意贬低。
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
r#"你是一个善于解读人际关系的聚会侦探小将，深谙中文互联网的 CP 文化和社交黑话。请分析以下群聊记录中群友之间的互动模式，挖出 2-6 组最有趣的关系（两人组合或多人团伙）。

要求：
- 每组关系需有明确成员名单（type 为 "duo" 或 "group"）。
- label 是 4-8 字的趣味小标签，要有网感，放得开。
  好例子："互损搭档"、"赛博老夫老妻"、"塑料姐妹花"、"单方面碰瓷天团"、"抬杠永动机"、"深夜emo互助组"、"复读机联盟"
  差例子：太正式或太抽象的标签，如"友好讨论组"、"技术交流组"——这种缺乏趣味
- vibe 是一句话点评关系的精髓，像写小作文一样生动。基于群聊中的实际互动，而非泛泛而谈。
  好例子："一个负责往坑里跳，一个负责在旁边鼓掌"、"互相嫌弃但每次聊天都秒回"、"你以为是对手，其实是嘴硬心软的塑料兄弟"
- evidence 是 1-3 条具体的聊天内容作为佐证，以 JSON 字符串数组形式返回。每条必须是原文中的原句，保持完整不加删改。格式为 "[HH:MM] 昵称: 内容"。
- 如果记录不够充分，就少归纳几组，不要强行凑数。
- 玩梗可以损但不能毒，底线是不人身攻击、不涉及敏感话题。

群聊记录：
{messages_text}

请严格返回如下 JSON 对象（纯 JSON，无 markdown 标记）：
{{
  "relationships": [
    {{
      "type": "duo",
      "members": ["成员1", "成员2"],
      "label": "互损搭档",
      "vibe": "互相嫌弃但每次聊天都秒回，标准口嫌体正直",
      "evidence": ["[22:30] 张三: 今晚通宵写代码吗？", "[22:31] 李四: 不了，明天还要早起搬砖。"]
    }}
  ]
}}"#
    )
}

// ── LLM 请求 ──────────────────────────────────────────────────────────────────

async fn call_llm(client: &LlmClient, prompt: &str) -> Result<String> {
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt,
    })];
    client.chat(&messages, 0.7, 4096).await
}

/// 清理 LLM 返回的原始文本，提取有效 JSON。
///
/// 处理步骤：
/// 1. 剥离 BOM 和零宽字符
/// 2. 剥离 markdown 代码块包裹
/// 3. 定位第一个 `[` 或 `{` 到最后一个 `]` 或 `}` 的 JSON 边界
/// 4. 移除 JSON 字符串值外部的不可见控制字符
fn clean_json(raw: &str) -> String {
    // Step 1: 剥离 BOM + 零宽字符
    let s: String = raw.chars().filter(|c| {
        !matches!(*c, '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FFFE}')
    }).collect();
    let s = s.trim();

    // Step 2: 剥离 markdown 代码块
    let s = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    let s = s.trim();

    // Step 3: 定位 JSON 边界 — 找第一个 [ 或 { 和最后一个 ] 或 }
    let json_str = {
        let first = s.find(|c: char| c == '[' || c == '{');
        let last = s.rfind(|c: char| c == ']' || c == '}');
        match (first, last) {
            (Some(f), Some(l)) if f <= l => &s[f..=l],
            _ => s,
        }
    };

    // Step 4: 移除 ASCII 控制字符（保留 \t \n \r，它们是 JSON 合法空白）
    json_str.chars().filter(|c| {
        !c.is_control() || matches!(*c, '\t' | '\n' | '\r')
    }).collect()
}

// ── 分析入口 ──────────────────────────────────────────────────────────────────

pub async fn analyze(messages: &[ChatMessage], client: &LlmClient) -> Result<LlmResult> {
    let formatted = format_for_llm(messages);
    debug!("[llm] 开始分析: {} 条消息, {}KB 文本", messages.len(), formatted.len() / 1024);

    // 如果消息文本太短，跳过 LLM
    if formatted.len() < 50 {
        debug!("[llm] 文本过短 ({}B), 跳过", formatted.len());
        return Ok(LlmResult::default());
    }

    // 截断保护: DeepSeek 128k 上下文, 保留尾部 ~150KB (≈50k 中文字符 ≈ 60-70k tokens)
    let truncated = if formatted.len() > 150_000 {
        let mut start = formatted.len() - 150_000;
        // 对齐到 UTF-8 char boundary，避免切到多字节字符中间
        while !formatted.is_char_boundary(start) { start += 1; }
        &formatted[start..]
    } else {
        &formatted
    };

    let mut result = LlmResult::default();

    // 三次 LLM 调用并行执行，大幅减少等待时间
    let topics_prompt        = build_topics_prompt(truncated);
    let titles_prompt        = build_titles_prompt(truncated);
    let relationships_prompt = build_relationships_prompt(truncated);

    let start = std::time::Instant::now();
    let (topics_res, titles_res, rel_res) = tokio::join!(
        call_llm(client, &topics_prompt),
        call_llm(client, &titles_prompt),
        call_llm(client, &relationships_prompt),
    );
    debug!("[llm] 三路并行完成, 耗时 {:.1}s", start.elapsed().as_secs_f64());

    // 解析话题结果
    match topics_res {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            // json_object 模式返回 {"topics": [...]}, 兼容裸数组
            let topics: Option<Vec<Topic>> = serde_json::from_str::<serde_json::Value>(&cleaned)
                .ok()
                .and_then(|v| {
                    if let Some(arr) = v.get("topics") {
                        serde_json::from_value(arr.clone()).ok()
                    } else {
                        serde_json::from_value(v).ok()
                    }
                });
            match topics {
                Some(t) => result.topics = t,
                None => warn!("话题 JSON 解析失败\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 话题分析失败: {e}"),
    }

    // 解析称号+金句结果
    match titles_res {
        Ok(raw) => {
            let cleaned = clean_json(&raw);
            match serde_json::from_str::<serde_json::Value>(&cleaned) {
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
            // json_object 模式返回 {"relationships": [...]}, 兼容裸数组
            let rels: Option<Vec<Relationship>> = serde_json::from_str::<serde_json::Value>(&cleaned)
                .ok()
                .and_then(|v| {
                    if let Some(arr) = v.get("relationships") {
                        serde_json::from_value(arr.clone()).ok()
                    } else {
                        serde_json::from_value(v).ok()
                    }
                });
            match rels {
                Some(r) => result.relationships = r,
                None => warn!("关系报告 JSON 解析失败\n原文: {cleaned}"),
            }
        }
        Err(e) => warn!("LLM 关系报告失败: {e}"),
    }
    debug!(
        "[llm] 结果: {} 话题, {} 称号, {} 金句, {} 关系",
        result.topics.len(),
        result.user_titles.len(),
        result.golden_quotes.len(),
        result.relationships.len(),
    );
    Ok(result)
}
