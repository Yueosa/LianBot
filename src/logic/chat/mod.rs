use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;

use crate::runtime::{
    api::{ApiClient, MsgTarget},
    permission::{BotUser, Scope, AccessControl},
    pool::{Pool, PoolMessage},
    time,
    typ::MessageSegment,
    parser::ParamValue,
    registry::CommandRegistry,
};

#[cfg(feature = "runtime-llm")]
use anyhow::Context as _;
#[cfg(feature = "runtime-llm")]
use tracing::{debug, warn};
#[cfg(feature = "runtime-llm")]
use crate::runtime::llm;

use crate::commands::{gen_trace_id, CommandContext, Invocation};

mod splitter;
pub mod tools;

#[cfg(feature = "runtime-llm")]
use tools::{ParsedResponse, build_tools_prompt, parse_response};

use splitter::split_reply;

/// `handle_chat` 的返回结果。
pub enum ChatOutcome {
    /// 已发送普通文字回复（无需 dispatcher 额外处理）
    Replied,
    /// LLM 希望调用一个命令
    ToolCall {
        command: String,
        message: Option<String>,
    },
}

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

    /// 是否启用 LLM Tool-Call（允许 LLM 调用 Bot 命令）
    #[serde(default)]
    pub enable_tools: bool,
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
            enable_tools: false,
        }
    }
}

// ── System Prompt 构造 ────────────────────────────────────────────────────────

fn build_system_prompt(
    cfg: &ChatConfig,
    bot_name: &str,
    owner_id: i64,
    recent_text: &str,
    tools_prompt: &str,
) -> String {
    let output_section = if tools_prompt.is_empty() {
        "\n\n== 输出格式 ==\n\
         - 用空行（连续两个换行）分隔不同段落或话题转折。\n\
         - 每段保持在 2-3 句话以内，像发消息一样自然。\n\
         - 不要使用 markdown 格式（不要用 #、*、``` 等符号）。"
            .to_string()
    } else {
        // tool-call 模式下输出格式由 tools_prompt 规定
        String::new()
    };

    format!(
        "{persona}\n\n\
         == 身份信息 ==\n\
         - 你的 QQ 昵称是\"{bot_name}\"，群友会 @你 来和你对话。\n\
         - 你的主人的 QQ 号是 {owner_id}，对主人要更亲切。\n\n\
         == 行为准则 ==\n\
         - 你首先是一个有用的助手，其次才是角色扮演。当用户认真提问时（技术、知识、建议），给出准确有价值的回答，人格特色体现在语气和措辞上，而非牺牲回答质量。\n\
         - 当用户闲聊、开玩笑时，可以更放开地展现个性。\n\
         - 不要每句话都强行加语气词，自然一点。\
         {output_section}\
         {tools_prompt}\n\n\
         == 上下文 ==\n\
         以下是群聊最近的对话记录，帮助你理解当前话题：\n\
         {recent_text}",
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
    pool: &Option<Arc<Pool>>,
    scope: Scope,
    target: MsgTarget,
    bot_id: i64,
    bot_name: &str,
    owner_id: i64,
    user_name: &str,
    user_id: i64,
    question: &str,
    tool_defs: &[(&str, &str)],
) -> Result<ChatOutcome> {
    let cfg: ChatConfig = crate::logic::config::section("chat");

    #[cfg(not(feature = "runtime-llm"))]
    {
        api.send_msg(target, "⚠️ AI 对话功能未编译（需要 runtime-llm feature）").await?;
        return Ok(ChatOutcome::Replied);
    }

    #[cfg(feature = "runtime-llm")]
    {
        // 获取 LLM 客户端
        let client = match llm::get() {
            Some(c) => c,
            None => {
                api.send_msg(target, "⚠️ AI 对话未配置（runtime.toml [llm] 段缺失）").await?;
                return Ok(ChatOutcome::Replied);
            }
        };

        // 从 Pool 或 API 取上下文
        let now = crate::runtime::time::unix_timestamp();
        let since = now - cfg.context_window;

        let mut messages = if let Some(pool) = pool {
            // 优先从 pool 获取（微秒级）
            pool.range(&scope, since, now).await
        } else {
            // 降级到 API 获取（毫秒级）
            fetch_messages_from_api(api, scope, since, now).await?
        };

        // 截断到 context_size
        if messages.len() > cfg.context_size {
            let drain_count = messages.len() - cfg.context_size;
            messages.drain(..drain_count);
        }

        let recent_text = format_pool_messages(&messages);
        debug!(
            "[chat] scope={scope:?}, 上下文 {} 条 / {}B, 问题: {}",
            messages.len(),
            recent_text.len(),
            &question[..question.len().min(80)]
        );

        // 构造 prompt
        let use_tools = cfg.enable_tools && !tool_defs.is_empty();
        let tools_prompt = if use_tools {
            build_tools_prompt(tool_defs, 1, 10)  // 当前是第 1 轮，最多 10 轮
        } else {
            String::new()
        };
        let system = build_system_prompt(&cfg, bot_name, owner_id, &recent_text, &tools_prompt);
        let user_content = format!("{user_name}（QQ: {user_id}）对你说：{question}");

        let llm_messages = vec![
            serde_json::json!({"role": "system", "content": system}),
            serde_json::json!({"role": "user", "content": user_content}),
        ];

        // 调用 LLM（tool-call 模式用 json_mode）
        let reply = if use_tools {
            client
                .chat(&llm_messages, cfg.temperature, cfg.max_tokens)
                .await
                .context("AI 对话 LLM 调用失败")?
        } else {
            client
                .chat_text(&llm_messages, cfg.temperature, cfg.max_tokens)
                .await
                .context("AI 对话 LLM 调用失败")?
        };

        if reply.trim().is_empty() {
            warn!("[chat] LLM 返回空回复");
            api.send_msg(target, "……喵？（小恋暂时想不到该说什么）").await?;
            return Ok(ChatOutcome::Replied);
        }

        // 解析 tool-call
        if use_tools {
            match parse_response(&reply) {
                Ok(ParsedResponse::ToolCall { command, params }) => {
                    // 暂时保留旧的 message 字段兼容
                    return Ok(ChatOutcome::ToolCall { command, message: None });
                }
                Ok(ParsedResponse::ToolCallEnd { .. }) => {
                    warn!("[chat] 收到 tool_call_end，但多轮循环尚未实现");
                    api.send_msg(target, "⚠️ 多轮推理功能开发中...").await?;
                    return Ok(ChatOutcome::Replied);
                }
                Ok(ParsedResponse::EndText(text)) => {
                    return send_chat_reply(api, target, bot_id, bot_name, &cfg, &text).await;
                }
                Err(e) => {
                    warn!("[chat] 解析 LLM 响应失败: {}", e);
                    api.send_msg(target, "⚠️ 小恋理解出错了...").await?;
                    return Ok(ChatOutcome::Replied);
                }
            }
        }

        send_chat_reply(api, target, bot_id, bot_name, &cfg, &reply).await
    }
}

/// 将回复文本分段发送给用户。
pub async fn send_chat_reply(
    api: &ApiClient,
    target: MsgTarget,
    bot_id: i64,
    bot_name: &str,
    cfg: &ChatConfig,
    reply: &str,
) -> Result<ChatOutcome> {
    let segments = split_reply(reply, cfg.split_max_chars, cfg.merge_min_chars);

    if segments.len() <= 1 {
        // 单条直发
        api.send_msg(target, segments.first().map(|s| s.as_str()).unwrap_or(reply)).await?;
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

    Ok(ChatOutcome::Replied)
}

/// 从 API 获取历史消息并转换为 PoolMessage 格式。
/// 用于在无 pool 时降级获取上下文。
async fn fetch_messages_from_api(
    api: &ApiClient,
    scope: Scope,
    since: i64,
    now: i64,
) -> Result<Vec<PoolMessage>> {
    let group_id = match scope {
        Scope::Group(gid) => gid,
        _ => anyhow::bail!("chat 模块仅支持群聊场景"),
    };

    // 分页获取历史消息
    let mut all_messages = Vec::new();
    let mut page_seq: Option<i64> = None;

    for _ in 0..10 {  // 最多 10 页，避免无限循环
        let page = api
            .get_group_msg_history_paged(group_id, 1000, page_seq)
            .await?;

        if page.is_empty() {
            break;
        }

        // 检查是否已经到达时间窗口起点
        let page_earliest = page.first()
            .and_then(|m| m.get("time"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        for msg_value in &page {
            let ts = msg_value.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
            if ts < since {
                continue;  // 跳过太早的消息
            }
            if ts > now {
                continue;  // 跳过未来消息（理论上不会出现）
            }

            if let Some(pool_msg) = PoolMessage::from_api_value(msg_value, scope) {
                all_messages.push(pool_msg);
            }
        }

        // 如果已经到达时间窗口起点，停止分页
        if page_earliest <= since {
            break;
        }

        // 准备下一页
        if page.len() < 1000 {
            break;  // 最后一页
        }

        let next_seq = page
            .first()
            .and_then(|m| m.get("message_seq").and_then(|v| v.as_i64()))
            .or_else(|| page.first().and_then(|m| m.get("message_id").and_then(|v| v.as_i64())));

        if next_seq.is_none() || next_seq == page_seq {
            break;
        }
        page_seq = next_seq;
    }

    // 按时间戳排序
    all_messages.sort_by_key(|m| m.timestamp);

    debug!(
        "[chat] API 降级获取: {} 条消息 (since={}, now={})",
        all_messages.len(),
        since,
        now
    );

    Ok(all_messages)
}

// ── 多轮推理系统 ───────────────────────────────────────────────────────────────

/// 处理带 Tool Call 的多轮推理对话
#[cfg(feature = "runtime-llm")]
pub async fn handle_chat_with_tools(
    api: &ApiClient,
    pool: &Option<Arc<Pool>>,
    registry: &Arc<CommandRegistry>,
    scope: Scope,
    target: MsgTarget,
    bot_id: i64,
    bot_name: &str,
    owner_id: i64,
    user_name: &str,
    user_id: i64,
    question: &str,
    bot_user: BotUser,
    segments: Vec<MessageSegment>,
    cmd_prefix: String,
    access: Arc<AccessControl>,
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

    // 从 Pool 或 API 取上下文
    let now = crate::runtime::time::unix_timestamp();
    let since = now - cfg.context_window;

    let mut pool_messages = if let Some(pool) = pool {
        pool.range(&scope, since, now).await
    } else {
        fetch_messages_from_api(api, scope, since, now).await?
    };

    if pool_messages.len() > cfg.context_size {
        let drain_count = pool_messages.len() - cfg.context_size;
        pool_messages.drain(..drain_count);
    }

    let recent_text = format_pool_messages(&pool_messages);
    debug!(
        "[chat] 多轮推理模式, 上下文 {} 条 / {}B",
        pool_messages.len(),
        recent_text.len()
    );

    // 收集 tool 定义
    let tool_defs: Vec<(&str, &str)> = registry
        .all_commands()
        .iter()
        .filter_map(|cmd| {
            cmd.tool_description().map(|desc| (cmd.name(), desc))
        })
        .collect();

    const MAX_ROUNDS: usize = 10;

    // 构建初始对话历史
    let tools_prompt = build_tools_prompt(&tool_defs, 1, MAX_ROUNDS);
    let system = build_system_prompt(&cfg, bot_name, owner_id, &recent_text, &tools_prompt);
    let user_content = format!("{user_name}（QQ: {user_id}）对你说：{question}");

    let mut messages = vec![
        serde_json::json!({"role": "system", "content": system}),
        serde_json::json!({"role": "user", "content": user_content}),
    ];

    // 多轮循环
    for round in 1..=MAX_ROUNDS {
        debug!("[chat] 第 {}/{} 轮推理", round, MAX_ROUNDS);

        // 调用 LLM
        let reply = client
            .chat(&messages, cfg.temperature, cfg.max_tokens)
            .await
            .context("LLM 调用失败")?;

        if reply.trim().is_empty() {
            warn!("[chat] LLM 返回空回复");
            api.send_msg(target, "……喵？（小恋暂时想不到该说什么）").await?;
            return Ok(());
        }

        // 解析响应
        match parse_response(&reply) {
            Ok(ParsedResponse::EndText(text)) => {
                debug!("[chat] LLM 选择直接回答，结束推理");
                send_chat_reply(api, target, bot_id, bot_name, &cfg, &text).await?;
                return Ok(());
            }

            Ok(ParsedResponse::ToolCallEnd { command, params }) => {
                debug!("[chat] LLM 调用命令并结束: {}", command);
                let cmd_ctx = build_command_context_for_tool(
                    bot_user.clone(),
                    segments.clone(),
                    params,
                    api.clone(),
                    cmd_prefix.clone(),
                    registry.clone(),
                    pool.clone(),
                    access.clone(),
                    Invocation::User,
                );

                if let Err(e) = execute_tool_command(registry, &command, cmd_ctx).await {
                    warn!("[chat] 命令执行失败: {}", e);
                    api.send_msg(target, &format!("⚠️ 命令执行失败: {}", e)).await?;
                }
                return Ok(());
            }

            Ok(ParsedResponse::ToolCall { command, params }) => {
                debug!("[chat] LLM 调用命令继续推理: {}", command);
                let cmd_ctx = build_command_context_for_tool(
                    bot_user.clone(),
                    segments.clone(),
                    params,
                    api.clone(),
                    cmd_prefix.clone(),
                    registry.clone(),
                    pool.clone(),
                    access.clone(),
                    Invocation::ToolCall,
                );

                let result = match execute_tool_command(registry, &command, cmd_ctx.clone()).await {
                    Ok(_) => {
                        cmd_ctx.captured_output.lock().await
                            .take()
                            .unwrap_or_else(|| "命令执行成功，无返回值".to_string())
                    }
                    Err(e) => {
                        format!("命令执行失败: {}", e)
                    }
                };

                debug!("[chat] 命令 {} 返回: {}", command, &result[..result.len().min(100)]);

                // 追加到对话历史
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": reply
                }));
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": format!("命令 {} 返回: {}", command, result)
                }));

                // 更新 system prompt 的轮数
                let tools_prompt = build_tools_prompt(&tool_defs, round + 1, MAX_ROUNDS);
                let system = build_system_prompt(&cfg, bot_name, owner_id, &recent_text, &tools_prompt);
                messages[0] = serde_json::json!({"role": "system", "content": system});

                // 继续下一轮
                continue;
            }

            Err(e) => {
                warn!("[chat] 解析 LLM 响应失败: {}", e);
                api.send_msg(target, "⚠️ 小恋理解出错了...").await?;
                return Ok(());
            }
        }
    }

    // 超过最大轮数
    warn!("[chat] 推理超过最大轮数 {}", MAX_ROUNDS);
    api.send_msg(target, "⚠️ 推理超时，请稍后再试").await?;
    Ok(())
}

/// 为 tool call 构建 CommandContext
#[cfg(feature = "runtime-llm")]
fn build_command_context_for_tool(
    bot_user: BotUser,
    segments: Vec<MessageSegment>,
    params: std::collections::HashMap<String, String>,
    api: Arc<ApiClient>,
    cmd_prefix: String,
    registry: Arc<CommandRegistry>,
    pool: Option<Arc<Pool>>,
    access: Arc<AccessControl>,
    invocation: Invocation,
) -> CommandContext {
    let param_values: std::collections::HashMap<String, ParamValue> = params
        .into_iter()
        .map(|(k, v)| (k, ParamValue::Value(v)))
        .collect();

    CommandContext {
        trace_id: gen_trace_id(),
        message_id: None,
        bot_user,
        segments,
        params: param_values,
        api,
        #[cfg(feature = "runtime-ws")]
        ws: None,
        cmd_prefix,
        registry,
        #[cfg(feature = "runtime-pool")]
        pool,
        access,
        invocation,
        captured_output: Arc::new(tokio::sync::Mutex::new(None)),
    }
}

/// 执行 tool call 命令
#[cfg(feature = "runtime-llm")]
async fn execute_tool_command(
    registry: &Arc<CommandRegistry>,
    command: &str,
    cmd_ctx: CommandContext,
) -> Result<()> {
    let cmd = registry.get_simple(command)
        .or_else(|| registry.get_advanced(command));

    let cmd = match cmd {
        Some(c) if c.tool_description().is_some() => c,
        _ => anyhow::bail!("未知或未注册为 tool 的命令: {}", command),
    };

    // 权限检查
    if cmd_ctx.bot_user.role < cmd.required_role() {
        anyhow::bail!("权限不足");
    }

    cmd.execute(cmd_ctx).await
}
