use anyhow::Result;
use async_trait::async_trait;

use crate::command::{Command, CommandContext};

pub struct HelpCommand;

#[async_trait]
impl Command for HelpCommand {
    fn name(&self) -> &str { "/help" }
    fn aliases(&self) -> Vec<&str> { vec!["/lhelp"] }
    fn help(&self) -> &str { "显示所有可用命令" }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        // 注册表的 help_text 会在 handler 层生成并传入，
        // 这里直接向群里发一条简洁的静态帮助（注册表通过 ctx 不便访问）。
        // 如需动态帮助，可将 help_text 预生成后存入 AppState 再传入 ctx。
        let text = "\
LianBot 命令列表
── 简单命令（以 / 开头）──
  /ping    测试在线
  /stalk   截取主人屏幕并发送到本群
  /help    显示本帮助

── 复杂命令（格式 <名称> [参数]）──
  <img>    发送图片
    无参数  随机发送ACG图片
    -u / --url <URL>   图片链接

参数格式示例：
  <img> -u https://example.com/a.png
  <img> --url=https://example.com/a.png";
        ctx.api.send_text(ctx.group_id, text).await
    }
}
