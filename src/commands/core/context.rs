use std::{collections::HashMap, sync::Arc};

use crate::runtime::permission::{AccessControl, BotUser};
use crate::runtime::{
    api::ApiClient,
    parser::ParamValue,
    pool::Pool,
    registry::CommandRegistry,
    typ::MessageSegment,
    ws::WsManager,
};

pub struct CommandContext {
    /// 触发命令的群号
    pub group_id: i64,
    /// 触发消息的 message_id（用于回复等操作，部分事件可能无此字段）
    pub message_id: Option<i64>,
    /// 发送者的虚拟用户对象（包含 user_id、role、status）
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
    /// 消息池（per-group 内存缓冲，可选）
    pub pool: Option<Arc<Pool>>,
    /// 准入控制（block/unblock、enable/disable 等管理操作）
    pub access: Arc<AccessControl>,
}

impl CommandContext {
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
