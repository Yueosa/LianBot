use std::time::Duration;

use tracing::{info, warn};

use super::{BotService, ServiceContext};

pub struct SchedulerService {
    ctx: ServiceContext,
}

impl SchedulerService {
    pub fn new(ctx: ServiceContext) -> Self {
        Self { ctx }
    }
}

/// 返回距下一个本地 00:00:00 的秒数
#[cfg(feature = "cmd-smy")]
fn secs_until_midnight() -> u64 {
    use chrono::Timelike;
    let passed = chrono::Local::now().num_seconds_from_midnight() as u64;
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
                runtime::plugin_config::PluginConfig,
            };

            let secs = secs_until_midnight();
            info!(
                "[scheduler] 每日 smy 日报已调度，距首次执行 {secs}s ({:.1}h)",
                secs as f64 / 3600.0
            );
            tokio::time::sleep(Duration::from_secs(secs)).await;

            loop {
                info!("[scheduler] 触发每日 smy 日报");

                let cfg = PluginConfig::global().get_section::<SmyPluginConfig>("smy");

                match &cfg.llm {
                    None => {
                        warn!("[scheduler] [smy].llm 未配置，跳过本次 AI 日报");
                    }
                    Some(llm_cfg) => {
                        let llm_cfg = llm_cfg.clone();
                        let groups = self.ctx.perm.enabled_groups();
                        info!("[scheduler] 共 {} 个群需要生成日报", groups.len());

                        for group_id in groups {
                            match run_smy_for_group(&self.ctx, group_id, &llm_cfg, cfg.screenshot_width).await {
                                Ok(()) => info!("[scheduler] 群 {group_id} 日报完成"),
                                Err(e) => warn!("[scheduler] 群 {group_id} 日报失败: {e:#}"),
                            }
                        }
                    }
                }

                info!("[scheduler] 本轮结束，等待 24h");
                tokio::time::sleep(Duration::from_secs(86400)).await;
            }
        }
    }
}

#[cfg(feature = "cmd-smy")]
async fn run_smy_for_group(
    ctx: &ServiceContext,
    group_id: i64,
    llm_config: &crate::kernel::config::LlmConfig,
    screenshot_width: u32,
) -> anyhow::Result<()> {
    use crate::logic::smy;

    let fetch_result = smy::fetcher::fetch(
        &ctx.api,
        &ctx.pool,
        group_id,
        Duration::from_secs(86400),
    )
    .await?;

    let messages = fetch_result.messages;
    if messages.is_empty() {
        info!("[scheduler] 群 {group_id} 今日无消息，跳过");
        return Ok(());
    }

    let stats = smy::statistics::analyze(&messages);

    let llm_result = match smy::llm::analyze(&messages, llm_config).await {
        Ok(r) => {
            info!("[scheduler] 群 {group_id} LLM 分析完成");
            r
        }
        Err(e) => {
            warn!("[scheduler] 群 {group_id} LLM 分析失败，使用空结果: {e:#}");
            smy::llm::LlmResult::default()
        }
    };

    let html = smy::renderer::render(&stats, &llm_result, "time=1d (定时日报)", &messages);

    let base64_img = smy::screenshot::capture(&html, screenshot_width).await?;
    ctx.api
        .send_image(group_id, &format!("base64://{base64_img}"))
        .await?;

    Ok(())
}
