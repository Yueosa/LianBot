use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::commands::{Command, CommandContext, CommandKind, ParamSpec, ParamKind, ValueConstraint};

#[cfg(feature = "runtime-permission")]
use crate::runtime::permission::Role;

#[cfg(feature = "runtime-api")]
use crate::runtime::api::MsgTarget;

pub struct ImgCommand;

#[async_trait]
impl Command for ImgCommand {
    fn name(&self) -> &str {
        "img"
    }

    fn kind(&self) -> CommandKind {
        CommandKind::Advanced
    }

    fn help(&self) -> &str {
        "批量发送图片（仅供管理员测试）"
    }

    fn declared_params(&self) -> &[ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                keys: &["-p", "--private"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: false,
                help: "私聊目标 QQ 号列表（逗号分隔）"
            },
            ParamSpec {
                keys: &["-g", "--group"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: false,
                help: "群聊目标群号列表（逗号分隔）"
            },
            ParamSpec {
                keys: &["-u", "--url"],
                kind: ParamKind::Value(ValueConstraint::Any),
                required: true,
                help: "图片 URL（仅支持 http/https）"
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
        // 解析 URL
        let url = ctx.get(&["-u", "--url"])
            .context("缺少 -u/--url 参数")?;

        // URL 安全验证
        validate_url(url)?;

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

        // 下载图片并转换为 base64
        let base64_url = download_and_encode_image(url).await
            .context("下载图片失败")?;

        // 统计
        let mut success_count = 0;
        let mut fail_count = 0;
        let mut errors = Vec::new();

        // 发送到私聊目标
        for user_id in private_targets {
            match ctx.api.send_image_to(MsgTarget::Private(user_id), &base64_url).await {
                Ok(_) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    errors.push(format!("私聊 {}: {}", user_id, e));
                }
            }
        }

        // 发送到群聊目标
        for group_id in group_targets {
            match ctx.api.send_image_to(MsgTarget::Group(group_id), &base64_url).await {
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
            for err in errors.iter().take(5) {
                result.push_str(&format!("  - {}\n", err));
            }
            if errors.len() > 5 {
                result.push_str(&format!("  ... 还有 {} 个错误\n", errors.len() - 5));
            }
        }

        ctx.reply(&result).await
    }
}

/// 验证 URL 安全性
fn validate_url(url: &str) -> Result<()> {
    // 必须是 http 或 https
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("仅支持 http:// 或 https:// 协议");
    }

    // 禁止危险协议
    let url_lower = url.to_lowercase();
    if url_lower.contains("file://") || url_lower.contains("ftp://") {
        anyhow::bail!("不支持的协议");
    }

    Ok(())
}

/// 解析逗号分隔的 ID 列表
fn parse_id_list(input: &str) -> Result<Vec<i64>> {
    input.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let id = s.parse::<i64>()
                .with_context(|| format!("无效的 ID: {}", s))?;

            // 验证 ID 范围
            if id >= 10000 && id <= 9999999999 {
                Ok(id)
            } else {
                anyhow::bail!("ID 超出有效范围 (10000-9999999999): {}", id)
            }
        })
        .collect()
}

/// 下载图片并转换为 base64 URL
async fn download_and_encode_image(url: &str) -> Result<String> {
    #[cfg(feature = "runtime-http")]
    {
        use base64::Engine;

        let client = crate::runtime::http::client();

        // 下载图片（30 秒超时）
        let resp = client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .with_context(|| format!("下载图片失败: {}", url))?;

        // 检查状态码
        if !resp.status().is_success() {
            anyhow::bail!("下载图片失败: HTTP {}", resp.status());
        }

        // 获取图片数据
        let bytes = resp.bytes().await
            .context("读取图片数据失败")?;

        // 检查大小（限制 10MB）
        if bytes.len() > 10 * 1024 * 1024 {
            anyhow::bail!("图片过大（超过 10MB）");
        }

        // 转换为 base64
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(format!("base64://{}", base64_data))
    }

    #[cfg(not(feature = "runtime-http"))]
    {
        anyhow::bail!("HTTP 客户端未启用（需要 runtime-http feature）")
    }
}
