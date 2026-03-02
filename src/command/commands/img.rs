use anyhow::Result;
use async_trait::async_trait;

use crate::command::{Command, CommandContext};

/// 默认随机图片 API
const DEFAULT_IMAGE_URL: &str = "https://www.loliapi.com/bg/";

pub struct ImgCommand;

#[async_trait]
impl Command for ImgCommand {
    fn name(&self) -> &str { "img" }
    fn help(&self) -> &str { "发送图片  -u/--url <图片链接>（可选）" }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        // 安全检查：拒绝手动指定群号
        if ctx.has(&["-i", "--id"]) {
            return ctx
                .api
                .send_text(ctx.group_id, "⚠️ 风险提示: 手动指定群聊 ID 功能已关闭")
                .await;
        }

        let url = ctx
            .get(&["-u", "--url"])
            .unwrap_or(DEFAULT_IMAGE_URL);

        ctx.api.send_image(ctx.group_id, url).await
    }
}
