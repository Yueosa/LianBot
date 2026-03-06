pub mod github;
pub mod scheduler;

use std::sync::Arc;

use crate::{
    kernel::config::Config,
    runtime::{api::ApiClient, permission::PermissionStore, pool::Pool},
};

/// 注入到所有 Service 的公共上下文（bot 自发行为，无 BotUser）
#[derive(Clone)]
pub struct ServiceContext {
    pub api: Arc<ApiClient>,
    pub perm: Arc<PermissionStore>,
    pub pool: Option<Arc<Pool>>,
    pub config: &'static Config,
}

/// 所有后台服务实现此 trait。
/// `run(self)` 消耗所有权，由调用方用 `tokio::spawn` 包装。
pub trait BotService: Send + 'static {
    fn name(&self) -> &'static str;
    async fn run(self) -> anyhow::Result<()>;
}
