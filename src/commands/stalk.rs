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
        Some("视奸主人屏幕：通过 WebSocket 向主人电脑发送截图请求，返回实时屏幕截图。适合想看主人屏幕上具体画面时调用，与 alive（查软件列表）不同")
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
