use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::command::{Command, CommandContext};

const ALIVE_URL: &str = "https://alive.yeastar.xin/api/status";

// ── 数据结构 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AliveResponse {
    devices: Vec<Device>,
    note:    Option<String>,
    privacy: bool,
    site:    Site,
}

#[derive(Debug, Deserialize)]
struct Site {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Device {
    show_name: String,
    status:    String,
    using:     bool,
}

// ── 命令实现 ──────────────────────────────────────────────────────────────────

pub struct AliveCommand;

#[async_trait]
impl Command for AliveCommand {
    fn name(&self) -> &str { "/alive" }
    fn help(&self) -> &str { "查看主人当前的设备在线状态" }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let resp = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?
            .get(ALIVE_URL)
            .send()
            .await
            .context("请求 alive API 失败")?
            .json::<AliveResponse>()
            .await
            .context("解析 alive API 响应失败")?;

        // 私密模式
        if resp.privacy {
            return ctx
                .api
                .send_text(ctx.group_id, "主人开启了私密模式哦！不能看！")
                .await;
        }

        let mut lines: Vec<String> = Vec::new();

        // 标题：site.name（或 note 作为补充）
        lines.push(resp.site.name.clone());
        if let Some(note) = &resp.note {
            if !note.is_empty() {
                lines.push(String::new());
                lines.push(note.clone());
            }
        }

        // 设备列表
        lines.push(String::new());
        lines.push("📱 活跃设备".to_string());

        if resp.devices.is_empty() {
            lines.push("  暂无在线设备".to_string());
        } else {
            for dev in &resp.devices {
                let status = if dev.using {
                    dev.status.clone()
                } else {
                    "设备不在线哦~".to_string()
                };
                lines.push(format!("· {}", dev.show_name));
                lines.push(format!("    {}", status));
            }
        }

        ctx.api.send_text(ctx.group_id, &lines.join("\n")).await
    }
}
