use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::commands::{Command, CommandContext, CommandKind};

const API_URL: &str = "https://api.ecylt.com/v1/world_60s";

#[derive(Debug, Deserialize)]
struct WorldResponse {
    data: Vec<String>,
}

pub struct WorldCommand;

#[async_trait]
impl Command for WorldCommand {
    fn name(&self) -> &str { "/world" }
    fn help(&self) -> &str { "60秒看世界：今日新闻速览" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let resp = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?
            .get(API_URL)
            .send()
            .await
            .context("请求 60s 看世界 API 失败")?
            .json::<WorldResponse>()
            .await
            .context("解析 60s 看世界响应失败")?;

        let text = format!(
            "📰 60秒看世界\n\n{}\n\n数据来源：api.ecylt.com",
            resp.data.join("\n")
        );

        ctx.api.send_text(ctx.group_id, &text).await
    }
}
