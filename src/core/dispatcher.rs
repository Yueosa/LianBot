use std::{collections::HashMap, sync::Arc};

use tracing::{debug, info, warn};

use crate::{
    commands::{Command, CommandContext, ParamKind, ParamSpec, ValueConstraint},
    core::{
        api::ApiClient,
        config::Config,
        parser::{CommandParser, ParsedCommand, ParamValue},
        registry::CommandRegistry,
        typ::{MessageEvent, OneBotEvent},
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
    config: &'static Config,
    api: Arc<ApiClient>,
    ws: Arc<WsManager>,
    registry: Arc<CommandRegistry>,
}

impl Dispatcher {
    pub fn new(
        config: &'static Config,
        api: Arc<ApiClient>,
        ws: Arc<WsManager>,
        registry: Arc<CommandRegistry>,
    ) -> Self {
        Self { config, api, ws, registry }
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

        // 2. 群白名单过滤
        if !self.config.bot.whitelist.contains(&group_id) {
            return Ok(());
        }

        // 3. 提取文本
        let text = event.full_text();
        info!("[群 {group_id}] {}: {text}", event.user_id);

        if text.is_empty() {
            return Ok(()); // 纯图片/语音等，跳过
        }

        // 4. 尝试解析命令
        match CommandParser::parse(&text) {
            Some(ParsedCommand::Simple { name, trailing }) => {
                self.dispatch_simple(group_id, event.user_id, name, trailing).await
            }
            Some(ParsedCommand::Advanced { name, params }) => {
                self.dispatch_advanced(group_id, event.user_id, name, params).await
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
        user_id: i64,
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
                    let unknown: Vec<_> = trailing.iter().filter(|t| t.starts_with('-')).collect();
                    let detail = if !unknown.is_empty() {
                        format!("❌ 未知参数: {}（输入 {name} -h 查看用法）", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                    } else {
                        format!("❌ {name} 是简单命令，不接受额外参数（输入 {name} -h 查看用法）")
                    };
                    return self.api.send_text(group_id, &detail).await;
                }
                let ctx = self.build_ctx(group_id, user_id, Default::default());
                cmd.execute(ctx).await
            }
            None => {
                debug!("未知简单命令: {name}");
                self.api.send_text(group_id, &format!("❓ 未知命令: {name}，输入 /help 查看命令列表")).await
            }
        }
    }

    // ── 复杂命令路由 ──────────────────────────────────────────────────────────

    async fn dispatch_advanced(
        &self,
        group_id: i64,
        user_id: i64,
        name: String,
        params: std::collections::HashMap<String, ParamValue>,
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
                let ctx = self.build_ctx(group_id, user_id, params);
                cmd.execute(ctx).await
            }
            None => {
                debug!("未知复杂命令: {name}");
                self.api.send_text(group_id, &format!("❓ 未知命令: <{name}>，输入 /help 查看命令列表")).await
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

    fn build_ctx(
        &self,
        group_id: i64,
        user_id: i64,
        params: std::collections::HashMap<String, ParamValue>,
    ) -> CommandContext {
        CommandContext {
            group_id,
            user_id,
            params,
            api: self.api.clone(),
            ws: self.ws.clone(),
            config: self.config,
            registry: self.registry.clone(),
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
        lines.push(format!("  {}{:<18}  {}  {}", keys, type_tag, req_tag, spec.help));
    }
    lines.join("\n")
}
