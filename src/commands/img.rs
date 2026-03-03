use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use tracing::warn;

use crate::commands::{Command, CommandContext, CommandKind, ParamKind, ParamSpec, ValueConstraint};

/// 默认随机图片 API（会 302 重定向到实际图片）
const DEFAULT_IMAGE_URL: &str = "https://www.loliapi.com/bg/";

pub struct ImgCommand;

/// 跟随重定向，返回最终落地 URL。
/// 失败时（网络错误、超时等）返回 None，调用方回退到直接发送原始 URL。
async fn resolve_final_url(url: &str) -> Option<String> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    let final_url = resp.url().to_string();
    if final_url != url {
        Some(final_url)
    } else {
        None
    }
}

#[async_trait]
impl Command for ImgCommand {
    fn name(&self) -> &str { "img" }
    fn help(&self) -> &str { "发送图片（无参数则随机 ACG，同时附带图片直链）" }
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

        match resolve_final_url(url).await {
            Some(final_url) => {
                ctx.api.send_text_image(ctx.group_id, &final_url, &final_url).await
            }
            None => {
                if url != DEFAULT_IMAGE_URL {
                    warn!("[img] 无法解析最终 URL，直接发送: {url}");
                }
                ctx.api.send_image(ctx.group_id, url).await
            }
        }
    }
}
