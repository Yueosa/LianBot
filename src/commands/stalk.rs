use anyhow::Result;
use async_trait::async_trait;
use tracing::debug;

use crate::commands::{Command, CommandContext, CommandKind, Dependency};

pub struct StalkCommand;

#[async_trait]
impl Command for StalkCommand {
    fn name(&self) -> &str { "stalk" }
    fn help(&self) -> &str { "截取主人当前屏幕并发到群里（查设备状态请用 /alive）" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }
    fn dependencies(&self) -> &[Dependency] { &[Dependency::Ws] }
    fn tool_description(&self) -> Option<&str> {
        Some("截取主人当前的电脑屏幕截图，看主人在做什么")
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        if !ctx.ws.has_clients().await {
            debug!("[stalk] 无 WS 客户端");
            return ctx
                .reply("主人没有在使用电脑哦 🖥️")
                .await;
        }

        // 广播截图请求到所有已连接的客户端
        debug!("[stalk] 广播截图请求");
        let scope = ctx.bot_user.scope;
        let payload = match scope {
            crate::runtime::permission::Scope::Group(gid) => format!("stalk:{gid}"),
            crate::runtime::permission::Scope::Private(uid) => format!("stalk:private:{uid}"),
        };
        ctx.ws.broadcast(payload).await;

        ctx.reply("📸 正在获取截图，请稍候...").await
    }
}
