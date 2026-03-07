use std::{collections::HashMap, sync::Arc};

use tracing::{debug, info, warn};

use crate::{
    commands::{Command, CommandContext, ParamKind, ParamSpec, ValueConstraint},
    runtime::{
        permission::{AccessControl, BotUser, Role, Scope, Status},
        api::ApiClient,
        parser::{CommandParser, ParsedCommand, ParamValue},
        pool::{MessagePool, MsgStatus, Pool, PoolMessage, ProcessRecord},
        registry::CommandRegistry,
        typ::{MessageEvent, MessageSegment, OneBotEvent},
        ws::WsManager,
    },
};

// ── Dispatcher ────────────────────────────────────────────────────────────────
//
// 事件分发器，是整个 Bot 的"大脑"入口：
//   1. 接收已反序列化的 OneBotEvent
//   2. 白名单过滤
//   3. 命令识别 → 命令路由
//   4. 非命令消息交给 `handle_plain`（关键词 / 未来 AI 对话入口）

pub struct Dispatcher {
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
        owner: i64,
        cmd_prefix: String,
        api: Arc<ApiClient>,
        ws: Arc<WsManager>,
        registry: Arc<CommandRegistry>,
        pool: Option<Arc<Pool>>,
        access: Arc<AccessControl>,
    ) -> Self {
        Self { owner, cmd_prefix, api, ws, registry, pool, access }
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
            OneBotEvent::Notice(_) => {
                debug!("收到 Notice 事件（暂未处理）");
            }
            OneBotEvent::MetaEvent(meta) => {
                // 心跳等元事件静默忽略
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
        // 1. 只处理群消息
        let group_id = match event.group_id {
            Some(id) if event.is_group() => id,
            _ => return Ok(()), // 私聊暂不处理
        };

        // 2. 群网关校验（该群是否对 Bot 开放）
        if !self.access.is_group_enabled(group_id) {
            return Ok(());
        }

        // 3. 写入消息池（群开着就忠实记录，与用户状态无关）
        if let Some(pool) = &self.pool {
            if let Some(pool_msg) = PoolMessage::from_event(&event, group_id) {
                pool.push(pool_msg).await;
            }
        }

        // 4. 构造 BotUser（内联身份解析）
        let bot_user = self.resolve_user(event.user_id, Scope::Group(group_id));

        // 5. 用户门控（Blocked 静默丢弃，不触发任何动作，但消息已入池）
        if bot_user.status == Status::Blocked {
            return Ok(());
        }

        // 6. 提取文本
        let text = event.full_text();
        info!("[群 {group_id}] {}: {text}", event.user_id);

        if text.is_empty() {
            return Ok(()); // 纯图片/语音等，跳过
        }

        // 7. 尝试解析命令
        let message_id = event.message_id;
        let segments = event.message.clone();

        match CommandParser::parse(&text, &self.cmd_prefix) {
            Some(ParsedCommand::Simple { name, trailing }) => {
                self.dispatch_simple(group_id, message_id, bot_user, segments, name, trailing).await
            }
            Some(ParsedCommand::Advanced { name, params }) => {
                self.dispatch_advanced(group_id, message_id, bot_user, segments, name, params).await
            }
            None => {
                // 非命令消息 → 关键词匹配 / AI 对话入口（可扩展）
                self.handle_plain(group_id, &event, &text).await
            }
        }
    }

    // ── 简单命令路由 ──────────────────────────────────────────────────────────

    async fn dispatch_simple(
        &self,
        group_id: i64,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        name: String,
        trailing: Vec<String>,
    ) -> anyhow::Result<()> {
        match self.registry.get_simple(&name) {
            Some(cmd) => {
                // --help → 完整帮助（简介 + 参数表，Simple 命令无参数时两者相同）
                if trailing.iter().any(|t| t == "--help") {
                    return self.api.send_text(group_id, &format_full_help(cmd.as_ref())).await;
                }
                // -h → 一行简介
                if trailing.iter().any(|t| t == "-h") {
                    let text = format!("{} — {}", cmd.name(), cmd.help());
                    return self.api.send_text(group_id, &text).await;
                }
                // Simple 命令不接受其他参数
                if !trailing.is_empty() {
                    let prefix = &self.cmd_prefix;
                    let unknown: Vec<_> = trailing.iter().filter(|t| t.starts_with('-')).collect();
                    let detail = if !unknown.is_empty() {
                        format!("❌ 未知参数: {}（输入 {prefix}{name} -h 查看用法）", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                    } else {
                        format!("❌ {prefix}{name} 是简单命令，不接受额外参数（输入 {prefix}{name} -h 查看用法）")
                    };
                    return self.api.send_text(group_id, &detail).await;
                }
                let ctx = self.build_ctx(group_id, message_id, bot_user, segments, Default::default());
                self.execute_and_mark(cmd, ctx, group_id, message_id).await
            }
            None => {
                debug!("未知简单命令: {name}");
                let prefix = &self.cmd_prefix;
                self.api.send_text(group_id, &format!("❓ 未知命令: {prefix}{name}，输入 {prefix}help 查看命令列表")).await
            }
        }
    }

    // ── 复杂命令路由 ──────────────────────────────────────────────────────────

    async fn dispatch_advanced(
        &self,
        group_id: i64,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        name: String,
        params: HashMap<String, ParamValue>,
    ) -> anyhow::Result<()> {
        match self.registry.get_advanced(&name) {
            Some(cmd) => {
                // --help → 完整帮助（简介 + 参数表）
                if params.contains_key("--help") {
                    return self.api.send_text(group_id, &format_full_help(cmd.as_ref())).await;
                }
                // -h → 一行简介
                if params.contains_key("-h") {
                    let text = format!("{} — {}", cmd.name(), cmd.help());
                    return self.api.send_text(group_id, &text).await;
                }
                // 参数校验（未知/必填/值约束）
                if let Err(detail) = validate_params(&params, cmd.declared_params()) {
                    let text = format!("❌ {detail}\n输入 <{}> --help 查看用法", cmd.name());
                    return self.api.send_text(group_id, &text).await;
                }
                let ctx = self.build_ctx(group_id, message_id, bot_user, segments, params);
                self.execute_and_mark(cmd, ctx, group_id, message_id).await
            }
            None => {
                debug!("未知复杂命令: {name}");
                let prefix = &self.cmd_prefix;
                self.api.send_text(group_id, &format!("❓ 未知命令: <{name}>，输入 {prefix}help 查看命令列表")).await
            }
        }
    }

    // ── 普通消息处理（关键词 / AI 预留入口） ──────────────────────────────────

    /// 非命令消息处理。
    /// 当前：空实现（静默）。
    /// 未来：接入关键词表、AI 自动对话等。
    async fn handle_plain(
        &self,
        _group_id: i64,
        _event: &MessageEvent,
        _text: &str,
    ) -> anyhow::Result<()> {
        // TODO: 关键词匹配、AI 自动对话
        Ok(())
    }

    // ── 辅助 ──────────────────────────────────────────────────────────────────
    /// 执行命令并向消息池报告处理结果。
    /// 计时 → 执行 → 根据 Ok/Err 生成 ProcessRecord → pool.mark()
    async fn execute_and_mark(
        &self,
        cmd: &Arc<dyn Command>,
        ctx: CommandContext,
        group_id: i64,
        message_id: Option<i64>,
    ) -> anyhow::Result<()> {
        let cmd_name = cmd.name().to_string();
        let start = std::time::Instant::now();

        let result = cmd.execute(ctx).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        // 向消息池报告处理状态（无 pool 或无 msg_id 时跳过）
        if let (Some(pool), Some(mid)) = (&self.pool, message_id) {
            let (status, error) = match &result {
                Ok(()) => (MsgStatus::Done, None),
                Err(e) => (MsgStatus::Error, Some(format!("{e:#}"))),
            };
            pool.mark(mid, group_id, status, ProcessRecord {
                command: cmd_name,
                duration_ms,
                error,
            }).await;
        }

        result
    }

    /// 内联身份解析：综合 owner 判断 + 准入控制黑名单 → 产出 BotUser。
    /// 未来 LLM 接入后可扩展为独立的 UserResolver。
    fn resolve_user(&self, user_id: i64, scope: Scope) -> BotUser {
        let role = if user_id == self.owner {
            Role::Owner
        } else {
            Role::Member
        };

        let status = if role == Role::Owner {
            // Owner 永远 Normal，不受黑名单影响
            Status::Normal
        } else if self.access.is_user_blocked(user_id, &scope) {
            Status::Blocked
        } else {
            Status::Normal
        };

        BotUser { user_id, scope, role, status }
    }

    fn build_ctx(
        &self,
        group_id: i64,
        message_id: Option<i64>,
        bot_user: BotUser,
        segments: Vec<MessageSegment>,
        params: HashMap<String, ParamValue>,
    ) -> CommandContext {
        CommandContext {
            group_id,
            message_id,
            bot_user,
            segments,
            params,
            api: self.api.clone(),
            ws: self.ws.clone(),
            cmd_prefix: self.cmd_prefix.clone(),
            registry: self.registry.clone(),
            pool: self.pool.clone(),
        }
    }
}

// ── 参数校验 ──────────────────────────────────────────────────────────────────

/// 校验 params 是否符合 specs 声明。返回第一条错误的用户可见文本。
fn validate_params(
    params: &HashMap<String, ParamValue>,
    specs: &[ParamSpec],
) -> Result<(), String> {
    // 收集所有已声明的 key
    let declared: std::collections::HashSet<&str> = specs.iter()
        .flat_map(|s| s.keys.iter().copied())
        .collect();

    // 1. 未知参数
    for key in params.keys() {
        if !declared.contains(key.as_str()) {
            return Err(format!("未知参数: {key}"));
        }
    }

    // 2. 必填参数
    for spec in specs {
        if spec.required && !spec.keys.iter().any(|k| params.contains_key(*k)) {
            return Err(format!("缺少必填参数: {}", spec.keys.join(" / ")));
        }
    }

    // 3. 值约束
    for spec in specs {
        if let ParamKind::Value(constraint) = spec.kind {
            for &key in spec.keys {
                if let Some(ParamValue::Value(s)) = params.get(key) {
                    match constraint {
                        ValueConstraint::Any => {}
                        ValueConstraint::Integer { min, max } => {
                            match s.parse::<i64>() {
                                Err(_) => return Err(format!("{key} 需要整数，收到: \"{s}\"")),
                                Ok(n) => {
                                    if let Some(lo) = min {
                                        if n < lo { return Err(format!("{key} 不能小于 {lo}，收到: {n}")); }
                                    }
                                    if let Some(hi) = max {
                                        if n > hi { return Err(format!("{key} 不能大于 {hi}，收到: {n}")); }
                                    }
                                }
                            }
                        }
                        ValueConstraint::OneOf(choices) => {
                            if !choices.contains(&s.as_str()) {
                                return Err(format!("{key} 仅支持: {}，收到: \"{s}\"", choices.join(" / ")));
                            }
                        }
                    }
                    break; // 只校验第一个命中的 key
                }
            }
        }
    }

    Ok(())
}

// ── 帮助文本生成 ───────────────────────────────────────────────────────────────

/// 完整帮助文本：一行简介 + 自动格式化的参数表（`--help` 触发）。
fn format_full_help(cmd: &dyn Command) -> String {
    let specs = cmd.declared_params();
    let header = format!("{} — {}", cmd.name(), cmd.help());
    if specs.is_empty() {
        return header;
    }
    let mut lines = vec![header, String::new(), "参数：".to_string()];
    for spec in specs {
        let keys = spec.keys.join(", ");
        let type_tag: String = match spec.kind {
            ParamKind::Flag => String::new(),
            ParamKind::Value(ValueConstraint::Any) => " <字符串>".into(),
            ParamKind::Value(ValueConstraint::Integer { min, max }) => match (min, max) {
                (Some(lo), Some(hi)) => format!(" <整数 {lo}-{hi}>"),
                (Some(lo), None)     => format!(" <整数 ≥{lo}>"),
                (None,     Some(hi)) => format!(" <整数 ≤{hi}>"),
                (None,     None)     => " <整数>".into(),
            },
            ParamKind::Value(ValueConstraint::OneOf(choices)) => {
                format!(" <{}>", choices.join("|"))
            }
        };
        let req_tag = if spec.required { "[必填]" } else { "[可选]" };
        let col = format!("{keys}{type_tag}");
        lines.push(format!("  {:<24}  {}  {}", col, req_tag, spec.help));
    }
    lines.join("\n")
}
