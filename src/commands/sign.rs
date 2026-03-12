use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind};
use crate::logic::yiban::YiBanConfig;
use crate::runtime::permission::Role;

// ── !!sign [账号名] — 触发签到 ────────────────────────────────────────────────

pub struct SignCommand;

#[async_trait]
impl Command for SignCommand {
    fn name(&self) -> &str { "sign" }

    fn help(&self) -> &str {
        "触发易班签到（仅限 Owner）\n\
         用法：\n\
         · sign            — 触发全部账号签到\n\
         · sign <账号名>    — 触发指定账号签到"
    }

    fn kind(&self) -> CommandKind { CommandKind::Simple }

    fn required_role(&self) -> Role { Role::Owner }

    fn accepts_trailing(&self) -> bool { true }

    fn tool_description(&self) -> Option<&str> {
        Some(
            "触发易班自动签到。\
             不带参数时触发全部账号；\
             传入账号名时触发指定账号。\
             仅限 Owner 使用。",
        )
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let args = ctx.get(&["_args"]).unwrap_or("").trim().to_string();
        let cfg = crate::logic::config::section::<YiBanConfig>("yiban");

        let account = if args.is_empty() { None } else { Some(args.as_str()) };
        let result = crate::services::yiban::trigger_sign(&cfg, account).await;
        info!("[sign] trigger account={:?}", account);
        ctx.reply(&result).await
    }
}

// ── !!sign-status — 查询最近一次签到状态 ─────────────────────────────────────

pub struct SignStatusCommand;

#[async_trait]
impl Command for SignStatusCommand {
    fn name(&self) -> &str { "sign-status" }

    fn help(&self) -> &str { "查询最近一次签到状态（仅限 Owner）" }

    fn kind(&self) -> CommandKind { CommandKind::Simple }

    fn required_role(&self) -> Role { Role::Owner }

    fn tool_description(&self) -> Option<&str> {
        Some("查询最近一次易班签到的结果和状态。仅限 Owner 使用。")
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let cfg = crate::logic::config::section::<YiBanConfig>("yiban");
        let result = crate::services::yiban::get_status(&cfg).await;
        info!("[sign-status] queried");
        ctx.reply(&result).await
    }
}
