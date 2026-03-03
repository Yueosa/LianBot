pub mod ping;
pub mod help;
pub mod img;
pub mod stalk;
pub mod smy;
pub mod alive;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::core::{
    api::ApiClient,
    config::Config,
    parser::ParamValue,
    ws::WsManager,
};

// ── Command Trait ─────────────────────────────────────────────────────────────

/// 所有命令实现此 trait。
#[async_trait]
pub trait Command: Send + Sync {
    /// 命令主名，如 `"img"`、`"/ping"`
    fn name(&self) -> &str;

    /// 别名列表（默认为空）
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }

    /// 单行帮助描述
    fn help(&self) -> &str;

    /// 执行命令
    async fn execute(&self, ctx: CommandContext) -> anyhow::Result<()>;
}

// ── 命令上下文 ─────────────────────────────────────────────────────────────────

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
}
