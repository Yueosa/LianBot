mod model;
mod access;

pub use model::{BotUser, Role, Scope, Status};
pub use access::AccessControl;

use serde::Deserialize;

/// Bot 运行时身份与权限配置（runtime.toml [bot]）
#[derive(Debug, Deserialize)]
pub struct BotConfig {
    /// Bot 主人的 QQ 号（唯一，最高权限，不受黑名单影响）
    #[serde(default)]
    pub owner: i64,
    /// 权限数据库文件路径，默认 "permissions.db"
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// 启动时导入 DB 的初始群列表（已有记录则跳过，不会覆盖）
    #[serde(default)]
    pub initial_groups: Vec<i64>,
    /// 静态黑名单（QQ 号列表），无 core-db 时作为 fallback
    #[serde(default)]
    pub blacklist: Vec<i64>,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            owner: 0,
            db_path: "permissions.db".to_string(),
            initial_groups: Vec::new(),
            blacklist: Vec::new(),
        }
    }
}

fn default_db_path() -> String { "permissions.db".to_string() }
