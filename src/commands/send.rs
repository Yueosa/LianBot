use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::commands::{Command, CommandContext, CommandKind, ParamSpec, ParamKind, ValueConstraint};
use crate::runtime::{permission::Role, typ::MessageSegment};

pub struct SendCommand;

#[async_trait]
impl Command for SendCommand {
    fn name(&self) -> &str {
        "send"
    }

    fn kind(&self) -> CommandKind {
        CommandKind::Advanced
    }

    fn help(&self) -> &str {
        "发送图文混合消息（仅供 LLM 调用）"
    }

    fn declared_params(&self) -> &[ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                keys: &["messages"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: true,
                help: "MessageSegment 数组的 JSON 字符串"
            },
        ];
        PARAMS
    }

    fn required_role(&self) -> Role {
        Role::Member  // 所有人可用，但实际只有 LLM 会调用
    }

    fn tool_description(&self) -> Option<&str> {
        Some(r#"发送消息给用户，支持文字、图片混合。
参数 messages 是 MessageSegment 数组的 JSON 字符串：
- 文字：{"type": "text", "data": {"text": "文字内容"}}
- 图片：{"type": "image", "data": {"file": "图片URL或base64://"}}

示例（注意 JSON 字符串需要转义引号）：
"[{\"type\":\"text\",\"data\":{\"text\":\"看这张图\"}},{\"type\":\"image\",\"data\":{\"file\":\"https://example.com/cat.jpg\"}},{\"type\":\"text\",\"data\":{\"text\":\"是不是很可爱？\"}}]"

注意：
1. messages 参数是 JSON 字符串，不是 JSON 对象
2. 字符串内的引号需要转义为 \"
3. 支持多个 text 和 image 混合，按顺序发送"#)
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        // 解析 messages 参数（JSON 字符串 -> Vec<MessageSegment>）
        let segments: Vec<MessageSegment> = ctx.get_json(&["messages"])
            .context("解析 messages 参数失败，请确保传入的是有效的 MessageSegment 数组 JSON 字符串")?;

        // 校验非空
        if segments.is_empty() {
            anyhow::bail!("messages 不能为空");
        }

        // 使用 CommandContext 提供的公开方法
        ctx.reply_segments(segments).await?;

        Ok(())
    }
}
