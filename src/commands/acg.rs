use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use crate::commands::{Command, CommandContext, CommandKind, http_client};
use crate::logic::config as logic_config;

// ── 插件配置 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AcgPluginConfig {
    #[serde(default = "AcgPluginConfig::default_url")]
    api_url: String,
}

impl AcgPluginConfig {
    fn default_url() -> String { "https://www.loliapi.com/bg/".into() }
}

impl Default for AcgPluginConfig {
    fn default() -> Self { Self { api_url: Self::default_url() } }
}

pub struct AcgCommand;

/// 跟随 302 重定向，拿到落地图片 URL
async fn resolve_final_url(url: &str) -> Option<String> {
    let resp = http_client().get(url).send().await.ok()?;
    let final_url = resp.url().to_string();
    if final_url != url { Some(final_url) } else { None }
}

#[async_trait]
impl Command for AcgCommand {
    fn name(&self) -> &str { "acg" }
    fn help(&self) -> &str { "随机返回一张二次元图片" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let cfg = logic_config::section::<AcgPluginConfig>("acg");
        let url = &cfg.api_url;
        match resolve_final_url(url).await {
            Some(final_url) => {
                debug!("[acg] 落地 URL: {final_url}");
                ctx.reply_text_image(&final_url, &final_url).await
            }
            None => ctx.reply_image(url).await,
        }
    }
}
