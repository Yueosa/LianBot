use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind};

const ACG_URL: &str = "https://www.loliapi.com/bg/";

pub struct AcgCommand;

/// 跟随 302 重定向，拿到落地图片 URL
async fn resolve_final_url(url: &str) -> Option<String> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    let final_url = resp.url().to_string();
    if final_url != url { Some(final_url) } else { None }
}

#[async_trait]
impl Command for AcgCommand {
    fn name(&self) -> &str { "acg" }
    fn help(&self) -> &str { "随机返回一张二次元图片" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        info!("[acg] 随机图, 群={}", ctx.group_id);
        match resolve_final_url(ACG_URL).await {
            Some(final_url) => {
                info!("[acg] 落地 URL: {final_url}");
                ctx.api.send_text_image(ctx.group_id, &final_url, &final_url).await
            }
            None => ctx.api.send_image(ctx.group_id, ACG_URL).await,
        }
    }
}
