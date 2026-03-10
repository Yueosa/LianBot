mod splitter;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::runtime::{
    api::{ApiClient, MsgTarget},
    llm,
    permission::Scope,
    pool::{MessagePool, Pool, PoolMessage},
    time,
    typ::MessageSegment,
};

use splitter::split_reply;

// ── 配置 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatConfig {
    /// 人格设定（支持多行），支持 {owner_name} 占位符
    #[serde(default = "ChatConfig::default_persona")]
    pub persona: String,

    /// 最多取几条 Pool 消息作为上下文
    #[serde(default = "ChatConfig::default_context_size")]
    pub context_size: usize,

    /// 最远回溯几秒（默认 2 小时）
    #[serde(default = "ChatConfig::default_context_window")]
    pub context_window: i64,

    /// LLM temperature（对话偏创意）
    #[serde(default = "ChatConfig::default_temperature")]
    pub temperature: f64,

    /// 单次回复 token 上限
    #[serde(default = "ChatConfig::default_max_tokens")]
    pub max_tokens: u32,

    /// 单段最大字符数（超过则按句切割）
    #[serde(default = "ChatConfig::default_split_max_chars")]
    pub split_max_chars: usize,

    /// 短于此字符数与上一段合并
    #[serde(default = "ChatConfig::default_merge_min_chars")]
    pub merge_min_chars: usize,

    /// 超过此段数用合并转发
    #[serde(default = "ChatConfig::default_forward_threshold")]
    pub forward_threshold: usize,

    /// 多条消息之间的发送间隔（毫秒）
    #[serde(default = "ChatConfig::default_send_delay_ms")]
    pub send_delay_ms: u64,
}

impl ChatConfig {
    fn default_persona() -> String {
        "你是小恋，一个17岁的猫娘少女，同时也是一个小魔女。\n\
         说话偶尔带喵，语气温柔但会傲娇。\n\
         你聪明、博学，对技术和日常话题都能给出有用的回答。"
            .to_string()
    }
    fn default_context_size() -> usize { 50 }
    fn default_context_window() -> i64 { 7200 }
    fn default_temperature() -> f64 { 0.8 }
    fn default_max_tokens() -> u32 { 2048 }
    fn default_split_max_chars() -> usize { 300 }
    fn default_merge_min_chars() -> usize { 20 }
    fn default_forward_threshold() -> usize { 5 }
    fn default_send_delay_ms() -> u64 { 600 }
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            persona: Self::default_persona(),
            context_size: Self::default_context_size(),
            context_window: Self::default_context_window(),
            temperature: Self::default_temperature(),
            max_tokens: Self::default_max_tokens(),
            split_max_chars: Self::default_split_max_chars(),
            merge_min_chars: Self::default_merge_min_chars(),
            forward_threshold: Self::default_forward_threshold(),
            send_delay_ms: Self::default_send_delay_ms(),
        }
    }
}

// ── System Prompt 构造 ────────────────────────────────────────────────────────

fn build_system_prompt(
    cfg: &ChatConfig,
    bot_name: &str,
    owner_id: i64,
    recent_text: &str,
) -> String {
    format!(
r#"{persona}

== 身份信息 ==
- 你的 QQ 昵称是"{bot_name}"，群友会 @你 来和你对话。
- 你的主人的 QQ 号是 {owner_id}，对主人要更亲切。

== 行为准则 ==
- 你首先是一个有用的助手，其次才是角色扮演。当用户认真提问时（技术、知识、建议），给出准确有价值的回答，人格特色体现在语气和措辞上，而非牺牲回答质量。
- 当用户闲聊、开玩笑时，可以更放开地展现个性。
- 不要每句话都强行加语气词，自然一点。

== 输出格式 ==
- 用空行（连续两个换行）分隔不同段落或话题转折。
- 每段保持在 2-3 句话以内，像发消息一样自然。
- 不要使用 markdown 格式（不要用 #、*、``` 等符号）。

== 上下文 ==
以下是群聊最近的对话记录，帮助你理解当前话题：
{recent_text}"#,
        persona = cfg.persona,
    )
}

// ── Pool 消息格式化（轻量版，直接用 PoolMessage） ─────────────────────────────

fn format_pool_messages(messages: &[PoolMessage]) -> String {
    let mut lines = Vec::with_capacity(messages.len());
    for msg in messages {
        let text = match &msg.text {
            Some(t) if !t.is_empty() => t.as_str(),
            _ => continue,
        };

        let time_str = time::from_timestamp(msg.timestamp)
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_else(|| "??:??".to_string());

        let name = if msg.is_bot {
            format!("[Bot]{}", msg.nickname)
        } else {
            msg.nickname.clone()
        };

        lines.push(format!("[{}] {}: {}", time_str, name, text));
    }
    lines.join("\n")
}

// ── 主入口 ─────────────────────────────────────────────────────────────────────

/// 处理一次 AI 对话请求。
///
/// `question` 是去掉 @Bot 之后的用户文本。
/// `user_name` 是提问者的昵称。
pub async fn handle_chat(
    api: &ApiClient,
    pool: &Arc<Pool>,
    scope: Scope,
    target: MsgTarget,
    bot_id: i64,
    bot_name: &str,
    owner_id: i64,
    user_name: &str,
    user_id: i64,
    question: &str,
) -> Result<()> {
    let cfg: ChatConfig = crate::logic::config::section("chat");

    // 获取 LLM 客户端
    let client = match llm::get() {
        Some(c) => c,
        None => {
            api.send_msg(target, "⚠️ AI 对话未配置（runtime.toml [llm] 段缺失）").await?;
            return Ok(());
        }
    };

    // 从 Pool 取上下文
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let since = now - cfg.context_window;

    let mut messages = pool.range(&scope, since, now).await;
    // 截断到 context_size
    if messages.len() > cfg.context_size {
        messages = messages.split_off(messages.len() - cfg.context_size);
    }

    let recent_text = format_pool_messages(&messages);
    debug!(
        "[chat] scope={scope:?}, 上下文 {} 条 / {}B, 问题: {}",
        messages.len(),
        recent_text.len(),
        &question[..question.len().min(80)]
    );

    // 构造 prompt
    let system = build_system_prompt(&cfg, bot_name, owner_id, &recent_text);
    let user_content = format!("{user_name}（QQ: {user_id}）对你说：{question}");

    let llm_messages = vec![
        serde_json::json!({"role": "system", "content": system}),
        serde_json::json!({"role": "user", "content": user_content}),
    ];

    // 调用 LLM
    let reply = client
        .chat_text(&llm_messages, cfg.temperature, cfg.max_tokens)
        .await
        .context("AI 对话 LLM 调用失败")?;

    if reply.trim().is_empty() {
        warn!("[chat] LLM 返回空回复");
        api.send_msg(target, "……喵？（小恋暂时想不到该说什么）").await?;
        return Ok(());
    }

    // 分段发送
    let segments = split_reply(&reply, cfg.split_max_chars, cfg.merge_min_chars);

    if segments.len() <= 1 {
        // 单条直发
        api.send_msg(target, segments.first().map(|s| s.as_str()).unwrap_or(&reply)).await?;
    } else if segments.len() <= cfg.forward_threshold {
        // 多条逐条发送
        for (i, seg) in segments.iter().enumerate() {
            api.send_msg(target, seg).await?;
            if i < segments.len() - 1 && cfg.send_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(cfg.send_delay_ms)).await;
            }
        }
    } else {
        // 合并转发
        let nodes: Vec<MessageSegment> = segments
            .iter()
            .map(|seg| MessageSegment::node(bot_id, bot_name, vec![MessageSegment::text(seg)]))
            .collect();
        api.send_forward_msg(target, nodes, None, Some("小恋的回复"), None).await?;
    }

    Ok(())
}
