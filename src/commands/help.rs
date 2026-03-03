use anyhow::Result;
use async_trait::async_trait;

use crate::commands::{Command, CommandContext, CommandKind};

pub struct HelpCommand;

#[async_trait]
impl Command for HelpCommand {
    fn name(&self) -> &str { "/help" }
    fn aliases(&self) -> &[&str] { &["/lhelp"] }
    fn help(&self) -> &str { "显示所有可用命令" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        ctx.api.send_text(ctx.group_id, &ctx.registry.help_text()).await
    }
}
