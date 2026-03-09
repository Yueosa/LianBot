use std::{collections::HashMap, hash::{BuildHasher, Hasher, RandomState}, sync::Arc};

use anyhow::Result;

use crate::runtime::permission::{AccessControl, BotUser, Scope};
use crate::runtime::{
    api::{ApiClient, MsgTarget},
    parser::ParamValue,
    pool::Pool,
    registry::CommandRegistry,
    typ::MessageSegment,
    ws::WsManager,
};

pub struct CommandContext {
    /// 本次命令执行的追踪标识（8 字符 hex），用于关联并发日志
    pub trace_id: String,
    /// 触发消息的 message_id（用于回复等操作，部分事件可能无此字段）
    pub message_id: Option<i64>,
    /// 发送者的虚拟用户对象（包含 user_id、scope、role）
    pub bot_user: BotUser,
    /// 原始消息段列表（含图片/at/回复等非文本 segment）
    pub segments: Vec<MessageSegment>,
    /// 解析后的参数 map
    pub params: HashMap<String, ParamValue>,
    /// OneBot API 客户端（Arc 共享）
    pub api: Arc<ApiClient>,
    /// WebSocket 连接管理器（Arc 共享）
    pub ws: Arc<WsManager>,
    /// 命令前缀（从 runtime 配置提取）
    pub cmd_prefix: String,
    /// 命令注册表（供 /help 等命令枚举全部命令）
    pub registry: Arc<CommandRegistry>,
    /// 消息池（per-scope 内存缓冲，可选）
    pub pool: Option<Arc<Pool>>,
    /// 准入控制（block/unblock、enable/disable 等管理操作）
    pub access: Arc<AccessControl>,
}

impl CommandContext {
    // ── Scope 便捷方法 ────────────────────────────────────────────────────────

    /// 当前交互域的发送目标（自动从 bot_user.scope 派生）。
    fn target(&self) -> MsgTarget {
        MsgTarget::from(self.bot_user.scope)
    }

    /// 若当前 scope 是群聊，返回 group_id；否则 None。
    pub fn group_id(&self) -> Option<i64> {
        match self.bot_user.scope {
            Scope::Group(gid) => Some(gid),
            _ => None,
        }
    }

    // ── 回复便捷方法 ─────────────────────────────────────────────────────────

    /// 向当前交互域发送纯文字回复。
    pub async fn reply(&self, text: &str) -> Result<()> {
        self.api.send_msg(self.target(), text).await
    }

    /// 向当前交互域发送图片。
    pub async fn reply_image(&self, file: &str) -> Result<()> {
        self.api.send_image_to(self.target(), file).await
    }

    /// 向当前交互域发送文字 + 图片（同一条消息）。
    pub async fn reply_text_image(&self, text: &str, file: &str) -> Result<()> {
        self.api.send_segments(self.target(), vec![
            MessageSegment::text(text),
            MessageSegment::image(file),
        ]).await
    }

    // ── 参数便捷方法 ─────────────────────────────────────────────────────────

    /// 按多个别名查找参数值字符串。
    /// 例如：`ctx.get(&["-u", "--url"])` 会按顺序尝试每个 key。
    pub fn get(&self, keys: &[&str]) -> Option<&str> {
        for &key in keys {
            if let Some(v) = self.params.get(key) {
                if let Some(s) = v.as_str() {
                    return Some(s);
                }
            }
        }
        None
    }
}

/// 生成 8 字符十六进制随机 trace_id，纯标准库实现，不引入外部依赖。
pub fn gen_trace_id() -> String {
    let h = RandomState::new().build_hasher().finish();
    format!("{:08x}", h as u32)
}
