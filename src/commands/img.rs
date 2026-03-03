use anyhow::Result;
use async_trait::async_trait;

use crate::commands::{Command, CommandContext, CommandKind, ParamKind, ParamSpec, ValueConstraint};

/// 默认随机图片 API
const DEFAULT_IMAGE_URL: &str = "https://www.loliapi.com/bg/";

pub struct ImgCommand;

#[async_trait]
impl Command for ImgCommand {
    fn name(&self) -> &str { "img" }
    fn help(&self) -> &str { "发送图片（无参数则随机 ACG）" }
    fn kind(&self) -> CommandKind { CommandKind::Advanced }
    fn declared_params(&self) -> &[ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec { keys: &["-u", "--url"], kind: ParamKind::Value(ValueConstraint::Any), required: false, help: "图片 URL（省略则随机 ACG）" },
        ];
        PARAMS
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let url = ctx
            .get(&["-u", "--url"])
            .unwrap_or(DEFAULT_IMAGE_URL);

        ctx.api.send_image(ctx.group_id, url).await
    }
}
