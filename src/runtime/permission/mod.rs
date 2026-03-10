mod model;
mod access;

pub use model::{BotUser, Role, Scope};
pub use access::AccessControl;

use serde::Deserialize;

/// Bot 运行时身份与权限配置（runtime.toml [bot]）
#[derive(Debug, Deserialize)]
pub struct BotConfig {
    /// Bot 自身的 QQ 号（用于检测 @Bot 等场景）
    #[serde(default)]
    pub bot_id: i64,
    /// Bot 主人的 QQ 号（唯一，最高权限，不受黑名单影响）
    #[serde(default)]
    pub owner: i64,
    /// 权限数据库文件路径，默认 "permissions.db"
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// 启动时导入 DB 的初始群列表（已有记录则跳过，不会覆盖）
    #[serde(default)]
    pub initial_groups: Vec<i64>,
    /// 群聊黑名单（QQ 号列表），Bot 忽略这些用户在任何群中的消息
    #[serde(default)]
    pub group_blacklist: Vec<i64>,
    /// 私聊黑名单（QQ 号列表），Bot 忽略这些用户的私聊消息
    #[serde(default)]
    pub private_blacklist: Vec<i64>,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            bot_id: 0,
            owner: 0,
            db_path: "permissions.db".to_string(),
            initial_groups: Vec::new(),
            group_blacklist: Vec::new(),
            private_blacklist: Vec::new(),
        }
    }
}

fn default_db_path() -> String { "permissions.db".to_string() }
