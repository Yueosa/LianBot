use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind};
use crate::logic::yiban::YiBanConfig;
use crate::runtime::permission::Role;

// ── sign — 易班签到命令（支持子命令） ─────────────────────────────────────────

pub struct SignCommand;

#[async_trait]
impl Command for SignCommand {
    fn name(&self) -> &str {
        "sign"
    }
    fn help(&self) -> &str {
        "触发易班签到（仅限 Owner）\n\
         用法：\n\
         · sign            — 触发全部用户签到\n\
         · sign <用户名>    — 触发指定用户签到\n\
         · sign status     — 查询最近一次签到状态"
    }
    fn kind(&self) -> CommandKind {
        CommandKind::Simple
    }
    fn required_role(&self) -> Role {
        Role::Owner
    }
    fn accepts_trailing(&self) -> bool {
        true
    }
    fn tool_description(&self) -> Option<&str> {
        Some(
            "触发易班自动签到。\
             不带参数时触发全部用户；\
             传入用户名时触发指定用户；\
             使用 status 子命令查询签到状态。\
             仅限 Owner 使用。",
        )
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let args = ctx.get(&["_args"]).unwrap_or("").trim();
        let cfg = crate::logic::config::section::<YiBanConfig>("yiban");

        // 子命令：status
        if args == "status" {
            let result = crate::services::yiban::get_status(&cfg).await;
            info!("[sign] status queried");
            return ctx.reply(&result).await;
        }

        // 主命令：触发签到
        // 设置 pending origin，Webhook 回调时额外推送到触发来源
        if let Some(bridge) = crate::services::yiban::bridge() {
            bridge.set_origin(ctx.scope());
        }

        let name = if args.is_empty() { None } else { Some(args) };
        let result = crate::services::yiban::trigger_sign(&cfg, name).await;
        info!("[sign] trigger name={:?}", name);
        ctx.reply(&result).await
    }
}
