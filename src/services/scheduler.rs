use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use super::BotService;
use crate::runtime::{api::ApiClient, permission::AccessControl, pool::Pool};

pub struct SchedulerService {
    api: Arc<ApiClient>,
    access: Arc<AccessControl>,
    pool: Option<Arc<Pool>>,
}

impl SchedulerService {
    pub fn new(
        api: Arc<ApiClient>,
        access: Arc<AccessControl>,
        pool: Option<Arc<Pool>>,
    ) -> Self {
        Self { api, access, pool }
    }
}

/// 返回距下一个配置时区 00:00:00 的秒数
#[cfg(feature = "cmd-smy")]
fn secs_until_midnight() -> u64 {
    use chrono::Timelike;
    let passed = crate::runtime::time::now().num_seconds_from_midnight() as u64;
    86400_u64.saturating_sub(passed)
}

impl BotService for SchedulerService {
    fn name(&self) -> &'static str {
        "scheduler"
    }

    async fn run(self) -> anyhow::Result<()> {
        info!("[{}] 已启动", self.name());

        // 若未编译 cmd-smy，scheduler 无任务，直接退出
        #[cfg(not(feature = "cmd-smy"))]
        {
            info!("[scheduler] cmd-smy 未启用，无定时任务");
            return Ok(());
        }

        #[cfg(feature = "cmd-smy")]
        {
            use crate::{
                logic::smy::SmyPluginConfig,
                runtime::logic_config,
            };

            let secs = secs_until_midnight();
            info!(
                "[scheduler] 每日 smy 日报已调度，距首次执行 {secs}s ({:.1}h)",
                secs as f64 / 3600.0
            );
            tokio::time::sleep(Duration::from_secs(secs)).await;

            loop {
                info!("[scheduler] 触发每日 smy 日报");

                let cfg = logic_config::section::<SmyPluginConfig>("smy");

                match &cfg.llm {
                    None => {
                        warn!("[scheduler] [smy].llm 未配置，跳过本次 AI 日报");
                    }
                    Some(llm_cfg) => {
                        let llm_cfg = llm_cfg.clone();
                        let groups = self.access.enabled_groups();
                        info!("[scheduler] 共 {} 个群需要生成日报", groups.len());

                        for group_id in groups {
                            match run_smy_for_group(&self.api, &self.pool, group_id, &llm_cfg, cfg.screenshot_width).await {
                                Ok(()) => info!("[scheduler] 群 {group_id} 日报完成"),
                                Err(e) => warn!("[scheduler] 群 {group_id} 日报失败: {e:#}"),
                            }
                        }
                    }
                }

                info!("[scheduler] 本轮结束，等待下一个午夜");
                let next = secs_until_midnight();
                info!("[scheduler] 距下次执行 {next}s ({:.1}h)", next as f64 / 3600.0);
                tokio::time::sleep(Duration::from_secs(next)).await;
            }
        }
    }
}

#[cfg(feature = "cmd-smy")]
async fn run_smy_for_group(
    api: &ApiClient,
    pool: &Option<Arc<Pool>>,
    group_id: i64,
    llm_config: &crate::logic::smy::LlmConfig,
    screenshot_width: u32,
) -> anyhow::Result<()> {
    use crate::logic::smy;

    let fetch_result = smy::fetcher::fetch(
        api,
        pool,
        group_id,
        Duration::from_secs(86400),
    )
    .await?;

    let messages = fetch_result.messages;
    if messages.is_empty() {
        info!("[scheduler] 群 {group_id} 今日无消息，跳过");
        return Ok(());
    }

    let base64_img = smy::generate_report(
        &messages,
        Some(llm_config),
        "time=1d (定时日报)",
        screenshot_width,
    ).await?;

    api
        .send_image(group_id, &format!("base64://{base64_img}"))
        .await?;

    Ok(())
}
