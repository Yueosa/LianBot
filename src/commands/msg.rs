use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::commands::{Command, CommandContext, CommandKind, ParamSpec, ParamKind, ValueConstraint};

#[cfg(feature = "runtime-permission")]
use crate::runtime::permission::Role;

#[cfg(feature = "runtime-api")]
use crate::runtime::api::MsgTarget;

pub struct MsgCommand;

#[async_trait]
impl Command for MsgCommand {
    fn name(&self) -> &str {
        "msg"
    }

    fn kind(&self) -> CommandKind {
        CommandKind::Advanced
    }

    fn help(&self) -> &str {
        "批量发送文本消息（仅供管理员测试）"
    }

    fn declared_params(&self) -> &[ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                keys: &["-p", "--private"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: false,
                help: "私聊目标 QQ 号列表（逗号分隔，如 123,456,789）"
            },
            ParamSpec {
                keys: &["-g", "--group"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: false,
                help: "群聊目标群号列表（逗号分隔）"
            },
            ParamSpec {
                keys: &["-m", "--message"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: true,
                help: "要发送的文本消息"
            },
        ];
        PARAMS
    }

    fn required_role(&self) -> Role {
        Role::Owner
    }

    fn tool_description(&self) -> Option<&str> {
        None  // 不暴露给 LLM
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        // 解析消息文本
        let message = ctx.get(&["-m", "--message"])
            .context("缺少 -m/--message 参数")?;

        // 解析目标列表
        let private_targets = if let Some(p) = ctx.get(&["-p", "--private"]) {
            parse_id_list(p).context("解析私聊目标失败")?
        } else {
            Vec::new()
        };

        let group_targets = if let Some(g) = ctx.get(&["-g", "--group"]) {
            parse_id_list(g).context("解析群聊目标失败")?
        } else {
            Vec::new()
        };

        // 至少需要一个目标
        if private_targets.is_empty() && group_targets.is_empty() {
            anyhow::bail!("至少需要指定一个目标（-p 或 -g）");
        }

        // 统计
        let mut success_count = 0;
        let mut fail_count = 0;
        let mut errors = Vec::new();

        // 发送到私聊目标
        for user_id in private_targets {
            match ctx.api.send_msg(MsgTarget::Private(user_id), message).await {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("私聊 {}: {}", user_id, e));
                }
            }
        }

        // 发送到群聊目标
        for group_id in group_targets {
            match ctx.api.send_msg(MsgTarget::Group(group_id), message).await {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("群 {}: {}", group_id, e));
                }
            }
        }

        // 返回结果
        let mut result = format!("✅ 发送完成：成功 {} 个，失败 {} 个", success_count, fail_count);
        if !errors.is_empty() {
            result.push_str("\n失败详情：\n");
            for err in errors.iter().take(5) {  // 最多显示 5 个错误
                result.push_str(&format!("  - {}\n", err));
            }
            if errors.len() > 5 {
                result.push_str(&format!("  ... 还有 {} 个错误\n", errors.len() - 5));
            }
        }

        ctx.reply(&result).await
    }
}

/// 解析逗号分隔的 ID 列表
fn parse_id_list(input: &str) -> Result<Vec<i64>> {
    input.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let id = s.parse::<i64>()
                .with_context(|| format!("无效的 ID: {}", s))?;

            // 验证 ID 范围（QQ 号/群号范围）
            if id >= 10000 && id <= 9999999999 {
                Ok(id)
            } else {
                anyhow::bail!("ID 超出有效范围 (10000-9999999999): {}", id)
            }
        })
        .collect()
}
