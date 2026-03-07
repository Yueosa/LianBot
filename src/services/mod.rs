#[cfg(feature = "svc-github")]
pub mod github;
pub mod scheduler;

/// 所有后台服务实现此 trait。
/// `run(self)` 消耗所有权，由调用方用 `tokio::spawn` 包装。
pub trait BotService: Send + 'static {
    fn name(&self) -> &'static str;
    async fn run(self) -> anyhow::Result<()>;
}

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 向 App 构建器注册所有后台服务和相关路由。
/// 各 Service 按需从 App 中获取依赖，不再使用统一的 ServiceContext。
pub fn register(app: &mut crate::kernel::app::App) {
    app.spawn(scheduler::SchedulerService::new(
        app.api.clone(),
        app.access.clone(),
        app.pool.clone(),
    ).run());

    #[cfg(feature = "svc-github")]
    github::register(app);
}
