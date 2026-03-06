//! GitHub Webhook 通知服务（薄管道）
//!
//! 业务逻辑（配置模型、验签、格式化）位于 `logic::github`。
//! 本模块仅负责：channel 消费 → 订阅匹配 → 群消息推送。

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::logic::github::{GitHubConfig, GitHubEvent, format_event};
use crate::runtime::api::MsgTarget;

use super::{BotService, ServiceContext};

// ── Service ───────────────────────────────────────────────────────────────────

pub struct GitHubService {
    rx: mpsc::Receiver<GitHubEvent>,
    ctx: ServiceContext,
    cfg: GitHubConfig,
}

impl GitHubService {
    pub fn new(rx: mpsc::Receiver<GitHubEvent>, ctx: ServiceContext, cfg: GitHubConfig) -> Self {
        Self { rx, ctx, cfg }
    }
}

impl BotService for GitHubService {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn run(mut self) -> anyhow::Result<()> {
        info!(
            "[{}] 已启动，共 {} 条订阅规则",
            self.name(),
            self.cfg.subscriptions.len()
        );

        while let Some(evt) = self.rx.recv().await {
            let Some(text) = format_event(&evt) else {
                continue;
            };

            // 找出所有匹配该事件的订阅
            let targets: Vec<(i64, Vec<i64>)> = self
                .cfg
                .subscriptions
                .iter()
                .filter(|s| s.matches(&evt.repo, &evt.event_type))
                .map(|s| (s.group, s.at.clone()))
                .collect();

            if targets.is_empty() {
                continue;
            }

            for (group_id, at_list) in targets {
                // 构造 @ 前缀
                let at_prefix: String = at_list
                    .iter()
                    .map(|qq| format!("@{qq} "))
                    .collect();
                let msg = format!("{at_prefix}{text}");

                if let Err(e) = self
                    .ctx
                    .api
                    .send_msg(MsgTarget::Group(group_id), &msg)
                    .await
                {
                    warn!("[github] 推送群 {group_id} 失败: {e:#}");
                }
            }
        }

        warn!("[{}] channel 已关闭，服务退出", self.name());
        Ok(())
    }
}
