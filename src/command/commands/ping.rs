use anyhow::Result;
use async_trait::async_trait;

use crate::command::{Command, CommandContext};

pub struct PingCommand;

#[async_trait]
impl Command for PingCommand {
    fn name(&self) -> &str { "/ping" }
    fn help(&self) -> &str { "测试 bot 是否在线" }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        ctx.api.send_text(ctx.group_id, "恋还活着哦! 🏓").await
    }
}
