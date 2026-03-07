use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use crate::commands::{Command, CommandContext, CommandKind, http_client};
use crate::runtime::logic_config;

// ── 插件配置 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WorldPluginConfig {
    #[serde(default = "WorldPluginConfig::default_url")]
    api_url: String,
}

impl WorldPluginConfig {
    fn default_url() -> String { "https://api.ecylt.com/v1/world_60s".into() }
}

impl Default for WorldPluginConfig {
    fn default() -> Self { Self { api_url: Self::default_url() } }
}

#[derive(Debug, Deserialize)]
struct WorldResponse {
    data: Vec<String>,
}

pub struct WorldCommand;

#[async_trait]
impl Command for WorldCommand {
    fn name(&self) -> &str { "world" }
    fn help(&self) -> &str { "60秒看世界：今日新闻速览" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let cfg = logic_config::section::<WorldPluginConfig>("world");
        info!("[world] 请求新闻, 群={}", ctx.group_id);
        let resp = http_client()
            .get(&cfg.api_url)
            .send()
            .await
            .context("请求 60s 看世界 API 失败")?
            .json::<WorldResponse>()
            .await
            .context("解析 60s 看世界响应失败")?;

        if resp.data.is_empty() {
            warn!("[world] 获取到空新闻列表, 群={}", ctx.group_id);
        } else {
            info!("[world] 获取 {} 条新闻, 群={}", resp.data.len(), ctx.group_id);
        }

        let text = format!(
            "📰 60秒看世界\n\n{}",
            resp.data.join("\n")
        );

        ctx.api.send_text(ctx.group_id, &text).await
    }
}
