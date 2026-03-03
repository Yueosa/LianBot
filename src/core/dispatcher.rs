use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::{
    commands::CommandContext,
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
            Some(ParsedCommand::Simple { name }) => {
                self.dispatch_simple(group_id, event.user_id, name).await
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
    ) -> anyhow::Result<()> {
        match self.registry.get_simple(&name) {
            Some(cmd) => {
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
                // 拦截 -h / --help → 显示命令帮助
                if params.contains_key("-h") || params.contains_key("--help") {
                    let help = format!("<{}> {}", cmd.name(), cmd.help());
                    return self.api.send_text(group_id, &help).await;
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
        }
    }
}
