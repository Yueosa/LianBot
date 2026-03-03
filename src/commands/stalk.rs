use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind, Dependency};

pub struct StalkCommand;

#[async_trait]
impl Command for StalkCommand {
    fn name(&self) -> &str { "/stalk" }
    fn help(&self) -> &str { "截取主人当前屏幕并发到群里（查设备状态请用 /alive）" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }
    fn dependencies(&self) -> &[Dependency] { &[Dependency::Ws] }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        if !ctx.ws.has_clients().await {
            info!("[stalk] 无 WS 客户端, 群={}", ctx.group_id);
            return ctx
                .api
                .send_text(ctx.group_id, "主人没有在使用电脑哦 🖥️")
                .await;
        }

        // 广播截图请求到所有已连接的客户端
        info!("[stalk] 广播截图请求, 群={}", ctx.group_id);
        ctx.ws
            .broadcast(format!("stalk:{}", ctx.group_id))
            .await;

        ctx.api
            .send_text(ctx.group_id, "📸 正在获取截图，请稍候...")
            .await
    }
}
