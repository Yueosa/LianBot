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
        let text = "\
LianBot 命令列表
── 简单命令（/ 开头）──
  /ping    测试在线
  /alive   查看主人设备在线状态
  /stalk   截取主人当前屏幕
  /help    显示本帮助

── 复杂命令（<名称> [参数]）──
  <img>    发送图片
  <smy>    群聊日报（加 -a 开启 AI 总结）

💡 输入 <命令> --help 查看详细用法
   例如: <img> --help";
        ctx.api.send_text(ctx.group_id, text).await
    }
}
