mod help;
mod validation;

use std::{collections::HashMap, sync::Arc};

use tracing::{debug, info, info_span, warn, Instrument};

use crate::{
    commands::{gen_trace_id, Command, CommandContext, Dependency},
    runtime::{
        permission::{AccessControl, BotUser, Role, Scope},
        api::{ApiClient, MsgTarget},
        parser::{CommandParser, ParsedCommand, ParamValue},
        pool::{MsgStatus, Pool, PoolMessage, ProcessRecord},
        registry::CommandRegistry,
        typ::{MessageEvent, MessageSegment, OneBotEvent},
        ws::WsManager,
    },
};

// ── Dispatcher ────────────────────────────────────────────────────────────────
//
// 事件分发器，是整个 Bot 的"大脑"入口：
//   1. 接收已反序列化的 OneBotEvent
//   2. 网关过滤（群白名单 / 私聊黑名单）
//   3. 命令识别 → 命令路由
//   4. 非命令消息交给 `handle_plain`（关键词 / 未来 AI 对话入口）

pub struct Dispatcher {
    bot_id: i64,
    owner: i64,
    cmd_prefix: String,
    api: Arc<ApiClient>,
    ws: Arc<WsManager>,
    registry: Arc<CommandRegistry>,
    pool: Option<Arc<Pool>>,
    access: Arc<AccessControl>,
}

impl Dispatcher {
    pub fn new(
        bot_id: i64,
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        ws: Arc<WsManager>,
        registry: Arc<CommandRegistry>,
        pool: Option<Arc<Pool>>,
        access: Arc<AccessControl>,
    ) -> Self {
        Self { bot_id, owner, cmd_prefix, api, ws, registry, pool, access }
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

    // ── 消息处理 ──────────────────────────────────────────────────────────────

    async fn handle_message(&self, event: MessageEvent) -> anyhow::Result<()> {
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
                    return Ok(());
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
        if let Some(pool) = &self.pool {
            if let Some(pool_msg) = PoolMessage::from_event(&event, scope, false) {
                pool.push(pool_msg).await;
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
            return Ok(());
        }

        // 6. 提取文本
        let desc = event.describe();
        let text = event.full_text();
        let scope_label = match scope {
            Scope::Group(gid) => format!("群 {gid}"),
            Scope::Private(uid) => format!("私聊 {uid}"),
        };
        info!("[{scope_label}] {}: {desc}", event.user_id);

        if text.is_empty() {
            return Ok(());
        }

        // 7. 尝试解析命令
        let message_id = event.message_id;
        let segments = event.message.clone();

        match CommandParser::parse(&text, &self.cmd_prefix) {
            Some(ParsedCommand::Simple { name, trailing }) => {
                self.dispatch_simple(scope, message_id, bot_user, segments, name, trailing).await
            }
            Some(ParsedCommand::Advanced { name, params }) => {
                self.dispatch_advanced(scope, message_id, bot_user, segments, name, params).await
            }
            None => {
                self.handle_plain(scope, &event, &text).await
            }
        }
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

        if let Some(pool) = &self.pool {
            if let Some(pool_msg) = PoolMessage::from_event(&event, scope, true) {
                pool.push(pool_msg).await;
            }
        }
    }

    // ── 简单命令路由 ──────────────────────────────────────────────────────────

    async fn dispatch_simple(
        &self,
        scope: Scope,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        name: String,
        trailing: Vec<String>,
    ) -> anyhow::Result<()> {
        let target = MsgTarget::from(scope);
        match self.registry.get_simple(&name) {
            Some(cmd) => {
                // 帮助请求
                if let Some(help_text) = help::try_help(cmd.as_ref(), |f| trailing.iter().any(|t| t == f)) {
                    return self.api.send_msg(target, &help_text).await;
                }
                // 依赖预检
                if let Some(msg) = self.check_dependencies(cmd.as_ref()).await {
                    return self.api.send_msg(target, &msg).await;
                }
                // 权限检查
                if bot_user.role < cmd.required_role() {
                    return self.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await;
                }
                // Simple 命令参数处理
                if cmd.accepts_trailing() {
                    let mut params: HashMap<String, ParamValue> = HashMap::new();
                    if !trailing.is_empty() {
                        params.insert("_args".into(), ParamValue::Value(trailing.join(" ")));
                    }
                    let ctx = self.build_ctx(message_id, bot_user, segments, params);
                    self.execute_and_mark(cmd, ctx, scope, message_id).await
                } else if !trailing.is_empty() {
                    let prefix = &self.cmd_prefix;
                    let unknown: Vec<_> = trailing.iter().filter(|t| t.starts_with('-')).collect();
                    let detail = if !unknown.is_empty() {
                        format!("❌ 未知参数: {}（输入 {prefix}{name} -h 查看用法）", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                    } else {
                        format!("❌ {prefix}{name} 是简单命令，不接受额外参数（输入 {prefix}{name} -h 查看用法）")
                    };
                    return self.api.send_msg(target, &detail).await;
                } else {
                    let ctx = self.build_ctx(message_id, bot_user, segments, Default::default());
                    self.execute_and_mark(cmd, ctx, scope, message_id).await
                }
            }
            None => {
                debug!("未知简单命令: {name}");
                let prefix = &self.cmd_prefix;
                self.api.send_msg(target, &format!("❓ 未知命令: {prefix}{name}，输入 {prefix}help 查看命令列表")).await
            }
        }
    }

    // ── 复杂命令路由 ──────────────────────────────────────────────────────────

    async fn dispatch_advanced(
        &self,
        scope: Scope,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        name: String,
        params: HashMap<String, ParamValue>,
    ) -> anyhow::Result<()> {
        let target = MsgTarget::from(scope);
        match self.registry.get_advanced(&name) {
            Some(cmd) => {
                // 帮助请求
                if let Some(help_text) = help::try_help(cmd.as_ref(), |f| params.contains_key(f)) {
                    return self.api.send_msg(target, &help_text).await;
                }
                // 依赖预检
                if let Some(msg) = self.check_dependencies(cmd.as_ref()).await {
                    return self.api.send_msg(target, &msg).await;
                }
                // 权限检查
                if bot_user.role < cmd.required_role() {
                    return self.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await;
                }
                // 参数校验
                if let Err(detail) = validation::validate_params(&params, cmd.declared_params()) {
                    let text = format!("❌ {detail}\n输入 <{}> --help 查看用法", cmd.name());
                    return self.api.send_msg(target, &text).await;
                }
                let ctx = self.build_ctx(message_id, bot_user, segments, params);
                self.execute_and_mark(cmd, ctx, scope, message_id).await
            }
            None => {
                debug!("未知复杂命令: {name}");
                let prefix = &self.cmd_prefix;
                self.api.send_msg(target, &format!("❓ 未知命令: <{name}>，输入 {prefix}help 查看命令列表")).await
            }
        }
    }

    // ── 普通消息处理（@Bot AI 对话） ──────────────────────────────────────────

    async fn handle_plain(
        &self,
        scope: Scope,
        event: &MessageEvent,
        _text: &str,
    ) -> anyhow::Result<()> {
        // 检测是否 @Bot
        if self.bot_id == 0 {
            return Ok(());
        }
        let is_at_bot = event.message.iter().any(|s| s.at_qq_id() == Some(self.bot_id));
        if !is_at_bot {
            return Ok(());
        }

        // 提取去掉 @段 的纯文本
        let question: String = event.message.iter()
            .filter(|s| !(s.is_at() && s.at_qq_id() == Some(self.bot_id)))
            .filter_map(|s| s.as_text())
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();

        if question.is_empty() {
            return Ok(());
        }

        // 用户昵称
        let user_name = event.sender.as_ref()
            .and_then(|s| {
                s.card.as_deref()
                    .filter(|c| !c.is_empty())
                    .or(s.nickname.as_deref())
            })
            .unwrap_or("未知");

        // Bot 昵称（如果 pool 里有 bot 的消息就能拿到，否则用默认）
        let bot_name = "小恋";

        let pool = match &self.pool {
            Some(p) => p,
            None => {
                let target = crate::runtime::api::MsgTarget::from(scope);
                self.api.send_msg(target, "⚠️ 消息池不可用，无法提供上下文").await?;
                return Ok(());
            }
        };

        let target = crate::runtime::api::MsgTarget::from(scope);

        // 收集 tool 定义（由 registry 中声明了 tool_description 的命令提供）
        let tool_defs = self.registry.tool_definitions();

        let outcome = crate::logic::chat::handle_chat(
            &self.api, pool, scope, target,
            self.bot_id, bot_name, self.owner,
            user_name, event.user_id, &question,
            &tool_defs,
        ).await?;

        // 处理 tool-call
        match outcome {
            crate::logic::chat::ChatOutcome::Replied => Ok(()),
            crate::logic::chat::ChatOutcome::ToolCall { command, message } => {
                if let Some(msg) = &message {
                    self.api.send_msg(target, msg).await?;
                }
                self.dispatch_tool_call(
                    scope,
                    event.message_id,
                    self.resolve_user(event.user_id, scope),
                    event.message.clone(),
                    &command,
                ).await
            }
        }
    }

    /// LLM tool-call 调用的命令路由。
    /// 仅匹配声明了 `tool_description()` 的命令，权限检查和依赖预检与普通命令一致。
    async fn dispatch_tool_call(
        &self,
        scope: Scope,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        command: &str,
    ) -> anyhow::Result<()> {
        let target = MsgTarget::from(scope);

        // 先查简单命令，再查复杂命令
        let cmd = self.registry.get_simple(command)
            .or_else(|| self.registry.get_advanced(command));

        let cmd = match cmd {
            Some(c) if c.tool_description().is_some() => c,
            _ => {
                warn!("[tool-call] LLM 调用了未知或未注册为 tool 的命令: {command}");
                return Ok(());
            }
        };

        // 依赖预检
        if let Some(msg) = self.check_dependencies(cmd.as_ref()).await {
            return self.api.send_msg(target, &msg).await;
        }
        // 权限检查
        if bot_user.role < cmd.required_role() {
            return self.api.send_msg(target, "⛔ 权限不足，该命令仅限 Bot 管理员使用").await;
        }

        let ctx = self.build_ctx(message_id, bot_user, segments, Default::default());
        self.execute_and_mark(cmd, ctx, scope, message_id).await
    }

    // ── 依赖预检 ────────────────────────────────────────────────────────────────

    async fn check_dependencies(&self, cmd: &dyn Command) -> Option<String> {
        for dep in cmd.dependencies() {
            let available = match dep {
                Dependency::Pool   => self.pool.is_some(),
                Dependency::Ws     => self.ws.has_clients().await,
                Dependency::Config => true,
            };
            if !available {
                let desc = match dep {
                    Dependency::Pool   => "消息池",
                    Dependency::Ws     => "WebSocket 连接",
                    Dependency::Config => "配置",
                };
                return Some(format!("⚠️ {} 需要{desc}，当前不可用", cmd.name()));
            }
        }
        None
    }

    // ── 辅助 ──────────────────────────────────────────────────────────────────

    async fn execute_and_mark(
        &self,
        cmd: &Arc<dyn Command>,
        ctx: CommandContext,
        scope: Scope,
        message_id: Option<i64>,
    ) -> anyhow::Result<()> {
        let cmd_name = cmd.name().to_string();
        let tid = ctx.trace_id.clone();
        let scope_label = match scope {
            Scope::Group(gid) => format!("{gid}"),
            Scope::Private(uid) => format!("p{uid}"),
        };
        let span = info_span!("cmd", tid = %tid, cmd = %cmd_name, scope = %scope_label);
        let start = std::time::Instant::now();

        let result = cmd.execute(ctx).instrument(span).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(()) => info!("[{cmd_name}] tid={tid} 完成 {duration_ms}ms"),
            Err(e) => {
                warn!("[{cmd_name}] tid={tid} 失败 {duration_ms}ms: {e:#}");
                // 统一向用户发送错误提示（尽力而为，失败不影响主流程）
                let target = MsgTarget::from(scope);
                let _ = self.api.send_msg(
                    target,
                    &format!("❌ 命令执行失败: {e}"),
                ).await;
            }
        }

        // 向消息池报告处理状态
        if let (Some(pool), Some(mid)) = (&self.pool, message_id) {
            let (status, error) = match &result {
                Ok(()) => (MsgStatus::Done, None),
                Err(e) => (MsgStatus::Error, Some(format!("{e:#}"))),
            };
            pool.mark(mid, &scope, status, ProcessRecord {
                command: cmd_name,
                duration_ms,
                error,
            }).await;
        }

        result
    }

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

    fn build_ctx(
        &self,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        params: HashMap<String, ParamValue>,
    ) -> CommandContext {
        CommandContext {
            trace_id: gen_trace_id(),
            message_id,
            bot_user,
            segments,
            params,
            api: self.api.clone(),
            ws: self.ws.clone(),
            cmd_prefix: self.cmd_prefix.clone(),
            registry: self.registry.clone(),
            pool: self.pool.clone(),
            access: self.access.clone(),
        }
    }
}
