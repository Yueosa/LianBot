use std::{collections::HashMap, sync::Arc};

use crate::{
    api::ApiClient,
    config::Config,
    parser::ParamValue,
    ws::WsManager,
};

// ── 命令上下文 ─────────────────────────────────────────────────────────────────
//
// 执行命令时传入的全部信息：来源、参数、及所有共享资源句柄。
// 命令实现只需持有 &CommandContext，不需要关心外部状态如何构建。

pub struct CommandContext {
    /// 触发命令的群号
    pub group_id: i64,
    /// 发送者 QQ 号
    pub user_id: i64,
    /// 解析后的参数 map
    pub params: HashMap<String, ParamValue>,
    /// OneBot API 客户端（Arc 共享）
    pub api: Arc<ApiClient>,
    /// WebSocket 连接管理器（Arc 共享）
    pub ws: Arc<WsManager>,
    /// 全局配置
    pub config: &'static Config,
}

impl CommandContext {
    // ── 参数查询 ──────────────────────────────────────────────────────────────

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

    /// 检查是否存在某个标志参数（Flag 或有值均视为存在）
    pub fn has(&self, keys: &[&str]) -> bool {
        keys.iter().any(|k| self.params.contains_key(*k))
    }
}
