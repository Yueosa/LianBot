#[cfg(feature = "svc-github")]
pub mod github;
pub mod scheduler;
#[cfg(feature = "svc-yiban")]
pub mod yiban;

/// 所有后台服务实现此 trait。
/// `run(self)` 消耗所有权，由调用方用 `tokio::spawn` 包装。
pub trait BotService: Send + 'static {
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    async fn run(self) -> anyhow::Result<()>;
}

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 服务注册摘要
#[derive(Default)]
pub struct ServicesSummary {
    /// 已注册的服务数量
    pub count: usize,
    /// 服务详细信息列表
    pub details: Vec<String>,
}

/// 向 App 构建器注册所有后台服务和相关路由。
/// 各 Service 按需从 App 中获取依赖，不再使用统一的 ServiceContext。
pub fn register(app: &mut crate::kernel::app::App) -> ServicesSummary {
    let mut summary = ServicesSummary::default();

    // scheduler 服务（需要 api, permission, pool）
    #[cfg(all(feature = "runtime-api", feature = "runtime-permission", feature = "runtime-pool"))]
    {
        app.spawn(scheduler::SchedulerService::new(
            app.api.clone().expect("runtime-api 未初始化"),
            app.access.clone().expect("runtime-permission 未初始化"),
            app.pool.clone(),
        ).run());
        summary.details.push("scheduler".to_string());
    }

    #[cfg(all(feature = "runtime-api", feature = "runtime-permission", not(feature = "runtime-pool")))]
    {
        app.spawn(scheduler::SchedulerService::new(
            app.api.clone().expect("runtime-api 未初始化"),
            app.access.clone().expect("runtime-permission 未初始化"),
        ).run());
        summary.details.push("scheduler".to_string());
    }

    #[cfg(feature = "svc-github")]
    {
        let gh_cfg = crate::logic::config::section::<crate::logic::github::GitHubConfig>("github");
        if !gh_cfg.secret.is_empty() {
            github::register(app);
            summary.details.push(format!("github ({} 条订阅)", gh_cfg.subscriptions.len()));
        }
    }

    #[cfg(feature = "svc-yiban")]
    {
        let yb_cfg = crate::logic::config::section::<crate::logic::yiban::YiBanConfig>("yiban");
        if !yb_cfg.targets.is_empty() {
            yiban::register(app);
            summary.details.push(format!("yiban ({} 条推送)", yb_cfg.targets.len()));
        }
    }

    summary.count = summary.details.len();
    summary
}
