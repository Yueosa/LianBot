mod help;
mod validation;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::{
    commands::{gen_trace_id, Command, CommandContext},
    runtime::{
        permission::{AccessControl, BotUser, Role, Scope},
        api::{ApiClient, MsgTarget},
        parser::ParamValue,
        registry::CommandRegistry,
        typ::{MessageEvent, MessageSegment, OneBotEvent},
    },
};

#[cfg(feature = "runtime-pool")]
use crate::runtime::pool::{MsgStatus, Pool, PoolMessage, ProcessRecord};

#[cfg(feature = "runtime-ws")]
use crate::runtime::ws::WsManager;

// ── ProcessRecord 占位符（无 runtime-pool 时使用） ──────────────────────────
#[cfg(not(feature = "runtime-pool"))]
#[derive(Debug, Clone)]
pub struct ProcessRecord;

// ── Handler Chain 架构 ────────────────────────────────────────────────────────

/// 消息上下文：在 handle_message 早期解析并结构化所有消息数据。
#[derive(Clone)]
pub struct MessageContext {
    /// 交互域（群聊或私聊）
    pub scope: Scope,
    /// 消息 ID
    pub message_id: Option<i64>,
    /// 用户身份
    pub bot_user: BotUser,
    /// 消息段（原始）
    pub segments: Vec<MessageSegment>,
    /// 完整文本（包含 @段）
    pub full_text: String,
    /// 用户昵称
    #[allow(dead_code)]
    pub user_name: String,
    /// 用户 ID
    #[allow(dead_code)]
    pub user_id: i64,
}

/// Handler 上下文：包装 MessageContext 并提供共享资源访问。
pub struct HandlerContext<'a> {
    pub msg: &'a mut MessageContext,
    pub api: &'a Arc<ApiClient>,
    #[cfg(feature = "runtime-ws")]
    pub ws: &'a Option<Arc<WsManager>>,
    pub registry: &'a Arc<CommandRegistry>,
    #[cfg(feature = "runtime-pool")]
    pub pool: &'a Option<Arc<Pool>>,
    pub access: &'a Arc<AccessControl>,
    pub bot_id: i64,
    #[allow(dead_code)]
    pub owner_id: i64,
    pub cmd_prefix: &'a str,
}

/// Handler 处理结果
pub enum HandlerResult {
    /// 已处理，停止后续 handler
    #[cfg(feature = "runtime-pool")]
    Handled(Option<ProcessRecord>),
    #[cfg(not(feature = "runtime-pool"))]
    Handled,
    /// 未处理，继续下一个 handler
    Continue,
    /// 跳过（如权限不足），停止后续 handler
    Skip,
}

/// 消息处理器 trait
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    /// O(1) 预过滤：快速判断是否需要处理此消息
    fn should_handle(&self, ctx: &MessageContext) -> bool;

    /// 实际处理逻辑
    async fn handle(&self, ctx: HandlerContext<'_>) -> Result<HandlerResult>;
}

// ── Dispatcher ────────────────────────────────────────────────────────────────
//
// 事件分发器，是整个 Bot 的"大脑"入口：
//   1. 接收已反序列化的 OneBotEvent
//   2. 网关过滤（群白名单 / 私聊黑名单）
//   3. 早期解析消息 → 构造 MessageContext
//   4. Handler Chain 顺序执行（命令、@Bot AI 对话等）

pub struct Dispatcher {
    bot_id: i64,
    owner: i64,
    cmd_prefix: String,
    api: Arc<ApiClient>,
    #[cfg(feature = "runtime-ws")]
    ws: Option<Arc<WsManager>>,
    registry: Arc<CommandRegistry>,
    #[cfg(feature = "runtime-pool")]
    pool: Option<Arc<Pool>>,
    access: Arc<AccessControl>,
    handlers: Vec<Box<dyn MessageHandler>>,
}

impl Dispatcher {
    #[cfg(all(feature = "runtime-ws", feature = "runtime-pool"))]
    pub fn new(
        bot_id: i64,
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        ws: Option<Arc<WsManager>>,
        registry: Arc<CommandRegistry>,
        pool: Option<Arc<Pool>>,
        access: Arc<AccessControl>,
    ) -> Self {
        let handlers: Vec<Box<dyn MessageHandler>> = vec![
            Box::new(CommandHandler),
            Box::new(AtBotHandler),
        ];
        Self { bot_id, owner, cmd_prefix, api, ws, registry, pool, access, handlers }
    }

    #[cfg(all(not(feature = "runtime-ws"), feature = "runtime-pool"))]
    pub fn new(
        bot_id: i64,
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        registry: Arc<CommandRegistry>,
        pool: Option<Arc<Pool>>,
        access: Arc<AccessControl>,
    ) -> Self {
        let handlers: Vec<Box<dyn MessageHandler>> = vec![
            Box::new(CommandHandler),
            Box::new(AtBotHandler),
        ];
        Self { bot_id, owner, cmd_prefix, api, registry, pool, access, handlers }
    }

    #[cfg(all(feature = "runtime-ws", not(feature = "runtime-pool")))]
    pub fn new(
        bot_id: i64,
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        ws: Option<Arc<WsManager>>,
        registry: Arc<CommandRegistry>,
        access: Arc<AccessControl>,
    ) -> Self {
        let handlers: Vec<Box<dyn MessageHandler>> = vec![
            Box::new(CommandHandler),
            Box::new(AtBotHandler),
        ];
        Self { bot_id, owner, cmd_prefix, api, ws, registry, access, handlers }
    }

    #[cfg(all(not(feature = "runtime-ws"), not(feature = "runtime-pool")))]
    pub fn new(
        bot_id: i64,
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        registry: Arc<CommandRegistry>,
        access: Arc<AccessControl>,
    ) -> Self {
        let handlers: Vec<Box<dyn MessageHandler>> = vec![
            Box::new(CommandHandler),
            Box::new(AtBotHandler),
        ];
        Self { bot_id, owner, cmd_prefix, api, registry, access, handlers }
    }

    // ── 顶层分发 ──────────────────────────────────────────────────────────────

    /// 分发一个 OneBot 事件。
    /// 所有错误在此层吸收，写日志但不上抛，保证 HTTP handler 始终返回 200。
    pub async fn dispatch(&self, event: OneBotEvent) {
        match event {
            OneBotEvent::Message(msg) => {
                if let Err(e) = self.handle_message(msg).await {
                    warn!("消息处理出错: {e:#}");
                }
            }
            OneBotEvent::MessageSent(msg) => {
                self.handle_bot_message(msg).await;
            }
            OneBotEvent::Notice(_) => {
                debug!("收到 Notice 事件（暂未处理）");
            }
            OneBotEvent::MetaEvent(meta) => {
                debug!("MetaEvent: {:?}", meta.meta_event_type);
            }
            OneBotEvent::Request(_) => {
                debug!("收到 Request 事件（暂未处理）");
            }
            OneBotEvent::Unknown => {
                debug!("收到未知事件类型");
            }
        }
    }

    // ── 消息处理（重构后的 Handler Chain 架构） ───────────────────────────────

    async fn handle_message(&self, event: MessageEvent) -> Result<()> {
        // 准备上下文（早期解析 + 网关过滤）
        let mut msg_ctx = match self.prepare_context(event).await? {
            Some(ctx) => ctx,
            None => return Ok(()), // 被网关拦截
        };

        // Handler Chain 顺序执行
        for handler in &self.handlers {
            if !handler.should_handle(&msg_ctx) {
                continue;
            }

            let scope = msg_ctx.scope;
            let message_id = msg_ctx.message_id;

            let handler_ctx = HandlerContext {
                msg: &mut msg_ctx,
                api: &self.api,
                #[cfg(feature = "runtime-ws")]
                ws: &self.ws,
                registry: &self.registry,
                #[cfg(feature = "runtime-pool")]
                pool: &self.pool,
                access: &self.access,
                bot_id: self.bot_id,
                owner_id: self.owner,
                cmd_prefix: &self.cmd_prefix,
            };

            match handler.handle(handler_ctx).await? {
                #[cfg(feature = "runtime-pool")]
                HandlerResult::Handled(record) => {
                    // 标记消息池
                    if let (Some(pool), Some(mid), Some(rec)) = (&self.pool, message_id, record) {
                        pool.mark(mid, &scope, MsgStatus::Done, rec).await;
                    }
                    return Ok(());
                }
                #[cfg(not(feature = "runtime-pool"))]
                HandlerResult::Handled => {
                    return Ok(());
                }
                HandlerResult::Skip => {
                    return Ok(());
                }
                HandlerResult::Continue => {
                    // 继续下一个 handler
                }
            }
        }

        Ok(())
    }

    /// 准备消息上下文：早期解析 + 网关过滤 + 写入消息池。
    /// 返回 None 表示被网关拦截。
    async fn prepare_context(&self, event: MessageEvent) -> Result<Option<MessageContext>> {
        // 1. 提取交互域
        let scope = if let Some(gid) = event.group_id.filter(|_| event.is_group()) {
            Scope::Group(gid)
        } else {
            Scope::Private(event.user_id)
        };

        let is_owner = event.user_id == self.owner;

        // 2. 网关校验（Owner 绕过）
        match scope {
            Scope::Group(gid) => {
                if !is_owner && !self.access.is_group_enabled(gid) {
                    return Ok(None);
                }
            }
            Scope::Private(_) => {
                // 私聊网关：当前仅 Owner 和非黑名单用户可通过
                // TODO: 后续可在 AccessControl 增加私聊白名单策略
            }
        }

        // 3. 写入消息池
        // 设计说明：
        //   - 群网关：控制是否记录整个群的消息（群级别开关）
        //   - 用户网关：控制用户是否能与机器人交互（用户级别开关，包括群聊@机器人、私聊等所有交互场景）
        //   - 消息池策略：通过群网关的消息如实记录，不受用户黑名单影响
        //     原因：群聊中被拉黑用户的消息仍是群上下文的一部分，对 AI 对话、日报生成等功能很重要
        //     用户黑名单仅阻止该用户触发命令执行，不影响消息记录
        #[cfg(feature = "runtime-pool")]
        if let Some(pool) = &self.pool {
            match PoolMessage::from_event(&event, scope, false) {
                Some(pool_msg) => pool.push(pool_msg).await,
                None => warn!("[dispatcher] PoolMessage::from_event 返回 None，可能存在协议兼容性问题: msg_id={:?}", event.message_id),
            }
        }

        // 4. 构造 BotUser
        let bot_user = self.resolve_user(event.user_id, scope);

        // 5. 用户门控（用户黑名单检查）
        // 用户网关管辖所有用户与机器人的交互场景：
        //   - 群聊中 @机器人 或触发命令
        //   - 私聊机器人
        // Owner 永远绕过黑名单检查
        if !is_owner && self.access.is_user_blocked(event.user_id, &scope) {
            return Ok(None);
        }

        // 6. 提取文本和用户信息
        let desc = event.describe();
        let full_text = event.full_text();
        let scope_label = match scope {
            Scope::Group(gid) => format!("群 {gid}"),
            Scope::Private(uid) => format!("私聊 {uid}"),
        };
        info!("[{scope_label}] {}: {desc}", event.user_id);

        if full_text.is_empty() {
            return Ok(None);
        }

        let user_name = event.sender.as_ref()
            .and_then(|s| {
                s.card.as_deref()
                    .filter(|c| !c.is_empty())
                    .or(s.nickname.as_deref())
            })
            .unwrap_or("未知")
            .to_string();

        Ok(Some(MessageContext {
            scope,
            message_id: event.message_id,
            bot_user,
            segments: event.message,
            full_text,
            user_name,
            user_id: event.user_id,
        }))
    }

    // ── Bot 自身消息处理（仅入池，不走命令路由） ──────────────────────────────

    async fn handle_bot_message(&self, event: MessageEvent) {
        let scope = if let Some(gid) = event.group_id.filter(|_| event.is_group()) {
            Scope::Group(gid)
        } else {
            Scope::Private(event.user_id)
        };

        if let Scope::Group(gid) = scope {
            if !self.access.is_group_enabled(gid) {
                return;
            }
        }

        #[cfg(feature = "runtime-pool")]
        if let Some(pool) = &self.pool {
            match PoolMessage::from_event(&event, scope, true) {
                Some(pool_msg) => pool.push(pool_msg).await,
                None => warn!("[dispatcher] Bot 消息 PoolMessage::from_event 返回 None: msg_id={:?}", event.message_id),
            }
        }
    }

    // ── 辅助 ──────────────────────────────────────────────────────────────────

    /// 内联身份解析：综合 owner 判断 → 产出 BotUser。
    /// 黑名单检查在网关层完成，此处不再判断 Status。
    fn resolve_user(&self, user_id: i64, scope: Scope) -> BotUser {
        let role = if user_id == self.owner {
            Role::Owner
        } else {
            Role::Member
        };

        BotUser { user_id, scope, role }
    }
}


// ── Concrete Handlers ─────────────────────────────────────────────────────────

/// 命令处理器：解析并执行命令
struct CommandHandler;

#[async_trait::async_trait]
impl MessageHandler for CommandHandler {
    fn should_handle(&self, ctx: &MessageContext) -> bool {
        // 命令以 cmd_prefix 开头，由 CommandParser 判断
        // 这里简单返回 true，实际判断在 handle 中进行
        !ctx.full_text.is_empty()
    }

    async fn handle(&self, ctx: HandlerContext<'_>) -> Result<HandlerResult> {
        use crate::runtime::parser::{CommandParser, ParsedCommand};

        match CommandParser::parse(&ctx.msg.full_text, ctx.cmd_prefix) {
            Some(ParsedCommand::Simple { name, trailing }) => {
                self.handle_simple(ctx, name, trailing).await
            }
            Some(ParsedCommand::Advanced { name, params }) => {
                self.handle_advanced(ctx, name, params).await
            }
            None => Ok(HandlerResult::Continue),
        }
    }
}

impl CommandHandler {
    async fn handle_simple(
        &self,
        ctx: HandlerContext<'_>,
        name: String,
        trailing: Vec<String>,
    ) -> Result<HandlerResult> {
        let target = MsgTarget::from(ctx.msg.scope);
        match ctx.registry.get_simple(&name) {
            Some(cmd) => {
                // 帮助请求
                if let Some(help_text) = help::try_help(cmd.as_ref(), |f| trailing.iter().any(|t| t == f)) {
                    ctx.api.send_msg(target, &help_text).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(None));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                }
                // 权限检查
                if ctx.msg.bot_user.role < cmd.required_role() {
                    ctx.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await?;
                    return Ok(HandlerResult::Skip);
                }
                // Simple 命令参数处理
                if cmd.accepts_trailing() {
                    let mut params: HashMap<String, ParamValue> = HashMap::new();
                    if !trailing.is_empty() {
                        params.insert("_args".into(), ParamValue::Value(trailing.join(" ")));
                    }
                    let cmd_ctx = build_command_ctx(ctx, params);
                    let record = execute_command(cmd, cmd_ctx).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(Some(record)));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                } else if !trailing.is_empty() {
                    let error_msg = build_trailing_error_msg(&trailing, ctx.cmd_prefix, &name);
                    ctx.api.send_msg(target, &error_msg).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(None));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                } else {
                    let cmd_ctx = build_command_ctx(ctx, Default::default());
                    let record = execute_command(cmd, cmd_ctx).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(Some(record)));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                }
            }
            None => {
                debug!("未知简单命令: {name}");
                ctx.api.send_msg(target, &format!("❓ 未知命令: {}{}，输入 {}help 查看命令列表",
                    ctx.cmd_prefix, name, ctx.cmd_prefix)).await?;
                #[cfg(feature = "runtime-pool")]
                return Ok(HandlerResult::Handled(None));
                #[cfg(not(feature = "runtime-pool"))]
                return Ok(HandlerResult::Handled);
            }
        }
    }

    async fn handle_advanced(
        &self,
        ctx: HandlerContext<'_>,
        name: String,
        params: HashMap<String, ParamValue>,
    ) -> Result<HandlerResult> {
        let target = MsgTarget::from(ctx.msg.scope);
        match ctx.registry.get_advanced(&name) {
            Some(cmd) => {
                // 帮助请求
                if let Some(help_text) = help::try_help(cmd.as_ref(), |f| params.contains_key(f)) {
                    ctx.api.send_msg(target, &help_text).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(None));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                }
                // 权限检查
                if ctx.msg.bot_user.role < cmd.required_role() {
                    ctx.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await?;
                    return Ok(HandlerResult::Skip);
                }
                // 参数校验
                if let Err(detail) = validation::validate_params(&params, cmd.declared_params()) {
                    let text = format!("❌ {detail}\n输入 <{}> --help 查看用法", cmd.name());
                    ctx.api.send_msg(target, &text).await?;
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(None));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                }
                let cmd_ctx = build_command_ctx(ctx, params);
                let record = execute_command(cmd, cmd_ctx).await?;
                #[cfg(feature = "runtime-pool")]
                return Ok(HandlerResult::Handled(Some(record)));
                #[cfg(not(feature = "runtime-pool"))]
                return Ok(HandlerResult::Handled);
            }
            None => {
                debug!("未知复杂命令: {name}");
                ctx.api.send_msg(target, &format!("❓ 未知命令: <{name}>，输入 {}help 查看命令列表",
                    ctx.cmd_prefix)).await?;
                #[cfg(feature = "runtime-pool")]
                return Ok(HandlerResult::Handled(None));
                #[cfg(not(feature = "runtime-pool"))]
                return Ok(HandlerResult::Handled);
            }
        }
    }
}

/// @Bot AI 对话处理器
struct AtBotHandler;

#[async_trait::async_trait]
impl MessageHandler for AtBotHandler {
    fn should_handle(&self, _ctx: &MessageContext) -> bool {
        // 需要在 handle 中检查 bot_id 和 @段，这里返回 true
        true
    }

    async fn handle(&self, ctx: HandlerContext<'_>) -> Result<HandlerResult> {
        // 检测是否 @Bot
        if ctx.bot_id == 0 {
            return Ok(HandlerResult::Continue);
        }

        let is_at_bot = ctx.msg.segments.iter().any(|s| s.at_qq_id() == Some(ctx.bot_id));
        if !is_at_bot {
            return Ok(HandlerResult::Continue);
        }

        // 提取去掉 @段 的纯文本
        let question: String = ctx.msg.segments.iter()
            .filter(|s| !(s.is_at() && s.at_qq_id() == Some(ctx.bot_id)))
            .filter_map(|s| s.as_text())
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();

        if question.is_empty() {
            return Ok(HandlerResult::Continue);
        }

        // Bot 昵称（如果 pool 里有 bot 的消息就能拿到，否则用默认）
        let bot_name = "小恋";

        let target = MsgTarget::from(ctx.msg.scope);

        // 收集 tool 定义（由 registry 中声明了 tool_description 的命令提供）
        let tool_defs = ctx.registry.tool_definitions();

        #[cfg(feature = "logic-chat")]
        {
            let outcome = crate::logic::chat::handle_chat(
                ctx.api,
                #[cfg(feature = "runtime-pool")]
                &ctx.pool,
                ctx.msg.scope, target,
                ctx.bot_id, bot_name, ctx.owner_id,
                &ctx.msg.user_name, ctx.msg.user_id, &question,
                &tool_defs,
            ).await?;

            // 处理 tool-call
            match outcome {
                crate::logic::chat::ChatOutcome::Replied => {
                    #[cfg(feature = "runtime-pool")]
                    return Ok(HandlerResult::Handled(None));
                    #[cfg(not(feature = "runtime-pool"))]
                    return Ok(HandlerResult::Handled);
                }
                crate::logic::chat::ChatOutcome::ToolCall { command, message } => {
                    if let Some(msg) = &message {
                        ctx.api.send_msg(target, msg).await?;
                    }
                    self.dispatch_tool_call(ctx, &command).await
                }
            }
        }

        #[cfg(not(feature = "logic-chat"))]
        {
            ctx.api.send_msg(target, "⚠️ AI 对话功能未编译（需要 logic-chat feature）").await?;
            #[cfg(feature = "runtime-pool")]
            return Ok(HandlerResult::Handled(None));
            #[cfg(not(feature = "runtime-pool"))]
            return Ok(HandlerResult::Handled);
        }
    }
}

impl AtBotHandler {
    /// LLM tool-call 调用的命令路由。
    /// 仅匹配声明了 `tool_description()` 的命令，权限检查和依赖预检与普通命令一致。
    #[allow(dead_code)]
    async fn dispatch_tool_call(
        &self,
        ctx: HandlerContext<'_>,
        command: &str,
    ) -> Result<HandlerResult> {
        let target = MsgTarget::from(ctx.msg.scope);

        // 先查简单命令，再查复杂命令
        let cmd = ctx.registry.get_simple(command)
            .or_else(|| ctx.registry.get_advanced(command));

        let cmd = match cmd {
            Some(c) if c.tool_description().is_some() => c,
            _ => {
                warn!("[tool-call] LLM 调用了未知或未注册为 tool 的命令: {command}");
                #[cfg(feature = "runtime-pool")]
                return Ok(HandlerResult::Handled(None));
                #[cfg(not(feature = "runtime-pool"))]
                return Ok(HandlerResult::Handled);
            }
        };

        // 权限检查
        if ctx.msg.bot_user.role < cmd.required_role() {
            ctx.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await?;
            return Ok(HandlerResult::Skip);
        }

        let cmd_ctx = build_command_ctx(ctx, Default::default());
        let record = execute_command(cmd, cmd_ctx).await?;
        #[cfg(feature = "runtime-pool")]
        return Ok(HandlerResult::Handled(Some(record)));
        #[cfg(not(feature = "runtime-pool"))]
        return Ok(HandlerResult::Handled);
    }
}

// ── Helper Functions ──────────────────────────────────────────────────────────

/// 构造简单命令的 trailing 参数错误提示。
/// 区分"未知参数"（以 - 开头）和"不接受参数"两种情况。
fn build_trailing_error_msg(trailing: &[String], cmd_prefix: &str, cmd_name: &str) -> String {
    let unknown: Vec<_> = trailing.iter().filter(|t| t.starts_with('-')).collect();
    if !unknown.is_empty() {
        format!(
            "❌ 未知参数: {}（输入 {}{} -h 查看用法）",
            unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            cmd_prefix,
            cmd_name
        )
    } else {
        format!(
            "❌ {}{} 是简单命令，不接受额外参数（输入 {}{} -h 查看用法）",
            cmd_prefix, cmd_name, cmd_prefix, cmd_name
        )
    }
}

fn build_command_ctx(
    ctx: HandlerContext<'_>,
    params: HashMap<String, ParamValue>,
) -> CommandContext {
    CommandContext {
        trace_id: gen_trace_id(),
        message_id: ctx.msg.message_id,
        bot_user: ctx.msg.bot_user.clone(),
        segments: ctx.msg.segments.clone(),
        params,
        api: ctx.api.clone(),
        #[cfg(feature = "runtime-ws")]
        ws: ctx.ws.clone(),
        cmd_prefix: ctx.cmd_prefix.to_string(),
        registry: ctx.registry.clone(),
        #[cfg(feature = "runtime-pool")]
        pool: ctx.pool.clone(),
        access: ctx.access.clone(),
    }
}

async fn execute_command(
    cmd: &Arc<dyn Command>,
    ctx: CommandContext,
) -> Result<ProcessRecord> {
    let cmd_name = cmd.name().to_string();
    let tid = ctx.trace_id.clone();
    let scope = ctx.bot_user.scope;
    let scope_label = match scope {
        Scope::Group(gid) => format!("{gid}"),
        Scope::Private(uid) => format!("p{uid}"),
    };
    let span = info_span!("cmd", tid = %tid, cmd = %cmd_name, scope = %scope_label);
    let start = std::time::Instant::now();

    let result = cmd.execute(ctx).instrument(span).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match &result {
        Ok(()) => {
            info!("[{cmd_name}] tid={tid} 完成 {duration_ms}ms");
            #[cfg(feature = "runtime-pool")]
            return Ok(ProcessRecord {
                command: cmd_name,
                duration_ms,
                error: None,
            });
            #[cfg(not(feature = "runtime-pool"))]
            return Ok(ProcessRecord);
        }
        Err(e) => {
            warn!("[{cmd_name}] tid={tid} 失败 {duration_ms}ms: {e:#}");
            #[cfg(feature = "runtime-pool")]
            return Ok(ProcessRecord {
                command: cmd_name,
                duration_ms,
                error: Some(format!("{e:#}")),
            });
            #[cfg(not(feature = "runtime-pool"))]
            return Ok(ProcessRecord);
        }
    }
}

