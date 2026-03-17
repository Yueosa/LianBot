use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind};

#[cfg(feature = "runtime-permission")]
use crate::runtime::permission::{Role, Scope};

pub struct AdminCommand;

#[async_trait]
impl Command for AdminCommand {
    fn name(&self) -> &str { "admin" }

    fn help(&self) -> &str {
        "管理命令\n\
         子命令：\n\
         · block @用户   — 拉黑用户（禁止私聊）\n\
         · unblock @用户 — 解除拉黑\n\
         · enable        — 启用当前群\n\
         · disable       — 禁用当前群"
    }

    fn kind(&self) -> CommandKind { CommandKind::Simple }

    fn required_role(&self) -> Role { Role::Owner }

    fn accepts_trailing(&self) -> bool { true }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let sub = ctx.get(&["_args"]).unwrap_or("").to_string();

        match sub.as_str() {
            "block" => {
                let target = ctx.segments.iter().find_map(|s| s.at_qq_id());
                match target {
                    Some(uid) => {
                        let target_scope = Scope::Private(uid);
                        if let Err(e) = ctx.access.block_user(&target_scope, uid).await {
                            return ctx.reply(&format!("❌ 拉黑失败: {e}")).await;
                        }
                        info!("[admin] block user={uid}");
                        ctx.reply(&format!("✅ 已拉黑用户 {uid}（私聊）")).await
                    }
                    None => ctx.reply("❌ 用法：!!admin block @用户").await,
                }
            }
            "unblock" => {
                let target = ctx.segments.iter().find_map(|s| s.at_qq_id());
                match target {
                    Some(uid) => {
                        let target_scope = Scope::Private(uid);
                        if let Err(e) = ctx.access.unblock_user(&target_scope, uid).await {
                            return ctx.reply(&format!("❌ 解除拉黑失败: {e}")).await;
                        }
                        info!("[admin] unblock user={uid}");
                        ctx.reply(&format!("✅ 已解除拉黑用户 {uid}")).await
                    }
                    None => ctx.reply("❌ 用法：!!admin unblock @用户").await,
                }
            }
            "enable" => {
                // 使用公开方法 group_id() 而不是直接访问 scope
                let gid = match ctx.group_id() {
                    Some(gid) => gid,
                    None => return ctx.reply("❌ 该命令仅在群聊中可用").await,
                };
                if let Err(e) = ctx.access.enable_group(gid).await {
                    return ctx.reply(&format!("❌ 启用群失败: {e}")).await;
                }
                info!("[admin] enable group={gid}");
                ctx.reply(&format!("✅ 已启用群 {gid}")).await
            }
            "disable" => {
                // 使用公开方法 group_id() 而不是直接访问 scope
                let gid = match ctx.group_id() {
                    Some(gid) => gid,
                    None => return ctx.reply("❌ 该命令仅在群聊中可用").await,
                };
                if let Err(e) = ctx.access.disable_group(gid).await {
                    return ctx.reply(&format!("❌ 禁用群失败: {e}")).await;
                }
                info!("[admin] disable group={gid}");
                ctx.reply(&format!("✅ 已禁用群 {gid}")).await
            }
            _ => {
                let prefix = &ctx.cmd_prefix;
                ctx.reply(
                    &format!("❌ 未知子命令，输入 {prefix}admin -h 查看用法"),
                ).await
            }
        }
    }
}
