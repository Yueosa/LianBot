#[cfg(feature = "svc-github")]
pub mod github;
pub mod scheduler;

use std::sync::Arc;

use crate::{
    runtime::{api::ApiClient, permission::AccessControl, pool::Pool},
};

/// 注入到所有 Service 的公共上下文（bot 自发行为，无 BotUser）
#[derive(Clone)]
pub struct ServiceContext {
    pub api: Arc<ApiClient>,
    pub access: Arc<AccessControl>,
    pub pool: Option<Arc<Pool>>,
}

/// 所有后台服务实现此 trait。
/// `run(self)` 消耗所有权，由调用方用 `tokio::spawn` 包装。
pub trait BotService: Send + 'static {
    fn name(&self) -> &'static str;
    async fn run(self) -> anyhow::Result<()>;
}

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 向 App 构建器注册所有后台服务和相关路由。
pub fn register(app: &mut crate::kernel::app::App) {
    let svc_ctx = ServiceContext {
        api: app.api.clone(),
        access: app.access.clone(),
        pool: app.pool.clone(),
    };

    app.spawn(scheduler::SchedulerService::new(svc_ctx.clone()).run());

    #[cfg(feature = "svc-github")]
    github::register(app, svc_ctx);
}
