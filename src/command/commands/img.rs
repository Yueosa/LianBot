use anyhow::Result;
use async_trait::async_trait;

use crate::command::{Command, CommandContext};

/// 默认随机图片 API
const DEFAULT_IMAGE_URL: &str = "https://www.loliapi.com/bg/";

pub struct ImgCommand;

#[async_trait]
impl Command for ImgCommand {
    fn name(&self) -> &str { "img" }
    fn help(&self) -> &str { "发送图片\n  无参数: 随机ACG图片\n  -u / --url <链接>: 指定图片URL\n\n示例:\n  <img>\n  <img> -u https://example.com/a.png\n  <img> --url=https://example.com/a.png" }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let url = ctx
            .get(&["-u", "--url"])
            .unwrap_or(DEFAULT_IMAGE_URL);

        ctx.api.send_image(ctx.group_id, url).await
    }
}
