use std::time::Duration;

use tracing::info;

use super::{BotService, ServiceContext};

pub struct SchedulerService {
    ctx: ServiceContext,
}

impl SchedulerService {
    pub fn new(ctx: ServiceContext) -> Self {
        Self { ctx }
    }
}

impl BotService for SchedulerService {
    fn name(&self) -> &'static str {
        "scheduler"
    }

    async fn run(self) -> anyhow::Result<()> {
        info!("[{}] 已启动，心跳间隔 60 s", self.name());
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            // TODO: Step 3 — 每日定时触发 smy 日报推送
            let _ = &self.ctx; // 占位，防止 unused 警告
        }
    }
}
