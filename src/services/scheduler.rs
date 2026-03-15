use std::sync::Arc;

use tracing::info;

use super::BotService;
use crate::runtime::{api::ApiClient, permission::AccessControl, pool::Pool};

pub struct SchedulerService {
    #[allow(dead_code)]
    api: Arc<ApiClient>,
    #[allow(dead_code)]
    access: Arc<AccessControl>,
    #[allow(dead_code)]
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

/// 返回距下一个配置时区指定小时（hh:00:00）的秒数。
/// 若当前已过该时间则返回距明天该时间的秒数。
#[allow(dead_code)]
fn secs_until_hour(hour: u32) -> u64 {
    use chrono::Timelike;
    let now = crate::runtime::time::now();
    let passed = now.num_seconds_from_midnight() as u64;
    let target = (hour as u64) * 3600;
    if passed < target {
        target - passed
    } else {
        86400 - passed + target
    }
}

impl BotService for SchedulerService {
    fn name(&self) -> &'static str {
        "scheduler"
    }

    async fn run(self) -> anyhow::Result<()> {
        info!("[{}] 已启动", self.name());

        // ── 任务 1：smy 日报（午夜触发） ─────────────────────────────────────
        #[cfg(feature = "cmd-smy")]
        {
            let api = self.api.clone();
            let access = self.access.clone();
            let pool = self.pool.clone();
            tokio::spawn(async move {
                let secs = secs_until_hour(0);
                info!(
                    "[scheduler/smy] 每日日报已调度，距首次执行 {secs}s ({:.1}h)",
                    secs as f64 / 3600.0
                );
                tokio::time::sleep(Duration::from_secs(secs)).await;

                loop {
                    run_smy_task(&api, &access, &pool).await;

                    let next = secs_until_hour(0);
                    info!("[scheduler/smy] 距下次执行 {next}s ({:.1}h)", next as f64 / 3600.0);
                    tokio::time::sleep(Duration::from_secs(next)).await;
                }
            });
        }

        // ── 任务 2：易班定时签到 ─────────────────────────────────────────────
        #[cfg(feature = "svc-yiban")]
        {
            let api = self.api.clone();
            tokio::spawn(async move {
                run_yiban_auto_sign_loop(&api).await;
            });
        }

        // 保持 service 存活（不退出，否则 App 会认为任务结束）
        std::future::pending::<()>().await;
        Ok(())
    }
}

// ── smy 日报任务 ──────────────────────────────────────────────────────────────

#[cfg(feature = "cmd-smy")]
async fn run_smy_task(api: &ApiClient, access: &AccessControl, pool: &Option<Arc<Pool>>) {
    use crate::{logic::smy::SmyPluginConfig, logic::config as logic_config};

    info!("[scheduler/smy] 触发每日日报");
    let cfg = logic_config::section::<SmyPluginConfig>("smy");

    #[cfg(feature = "runtime-llm")]
    let with_ai = crate::runtime::llm::get().is_some();

    #[cfg(not(feature = "runtime-llm"))]
    let with_ai = false;

    if !with_ai {
        warn!("[scheduler/smy] [llm] 未配置，日报将使用纯统计模式");
    }

    let groups = access.enabled_groups();
    info!("[scheduler/smy] 共 {} 个群需要生成日报 (ai={})", groups.len(), with_ai);

    for group_id in groups {
        match run_smy_for_group(api, pool, group_id, with_ai, cfg.screenshot_width).await {
            Ok(()) => info!("[scheduler/smy] 群 {group_id} 日报完成"),
            Err(e) => warn!("[scheduler/smy] 群 {group_id} 日报失败: {e:#}"),
        }
    }
    info!("[scheduler/smy] 本轮结束");
}

#[cfg(feature = "cmd-smy")]
async fn run_smy_for_group(
    api: &ApiClient,
    pool: &Option<Arc<Pool>>,
    group_id: i64,
    with_ai: bool,
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
        info!("[scheduler/smy] 群 {group_id} 今日无消息，跳过");
        return Ok(());
    }

    let base64_img = smy::generate_report(
        &messages,
        with_ai,
        "time=1d (定时日报)",
        screenshot_width,
    ).await?;

    api
        .send_image_to(MsgTarget::Group(group_id), &format!("base64://{base64_img}"))
        .await?;

    Ok(())
}

// ── 易班定时签到任务 ──────────────────────────────────────────────────────────

#[cfg(feature = "svc-yiban")]
async fn run_yiban_auto_sign_loop(api: &ApiClient) {
    use crate::logic::yiban::YiBanConfig;
    use crate::runtime::permission::BotConfig;

    let cfg = crate::logic::config::section::<YiBanConfig>("yiban");
    let Some(hour) = cfg.auto_sign_hour else {
        info!("[scheduler/yiban] auto_sign_hour 未配置，定时签到已禁用");
        return;
    };

    if hour > 23 {
        warn!("[scheduler/yiban] auto_sign_hour={hour} 无效（应为 0-23），已禁用");
        return;
    }

    let bot_cfg: BotConfig = crate::runtime::config::section("bot");
    let owner = bot_cfg.owner;

    let secs = secs_until_hour(hour as u32);
    info!(
        "[scheduler/yiban] 每日 {hour}:00 自动签到已调度，距首次执行 {secs}s ({:.1}h)",
        secs as f64 / 3600.0
    );
    tokio::time::sleep(Duration::from_secs(secs)).await;

    loop {
        info!("[scheduler/yiban] 触发每日自动签到");

        let cfg = crate::logic::config::section::<YiBanConfig>("yiban");
        let result = crate::services::yiban::trigger_sign(&cfg, None).await;

        if result.contains("失败") || result.contains("无法连接") {
            warn!("[scheduler/yiban] 自动签到异常: {result}");
            if owner != 0 {
                let msg = format!("⚠️ 每日自动签到异常\n{result}");
                if let Err(e) = api.send_msg(MsgTarget::Private(owner), &msg).await {
                    warn!("[scheduler/yiban] 通知 owner 失败: {e:#}");
                }
            }
        } else {
            info!("[scheduler/yiban] {result}");
        }

        let next = secs_until_hour(hour as u32);
        info!("[scheduler/yiban] 距下次执行 {next}s ({:.1}h)", next as f64 / 3600.0);
        tokio::time::sleep(Duration::from_secs(next)).await;
    }
}
