use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use crate::commands::{Command, CommandContext, CommandKind};
use crate::core::plugin_config::PluginConfig;

// ── 插件配置 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AlivePluginConfig {
    #[serde(default = "AlivePluginConfig::default_url")]
    api_url: String,
    #[serde(default = "AlivePluginConfig::default_timeout")]
    timeout_secs: u64,
}

impl AlivePluginConfig {
    fn default_url() -> String { "https://alive.yeastar.xin/api/status".into() }
    fn default_timeout() -> u64 { 5 }
}

impl Default for AlivePluginConfig {
    fn default() -> Self {
        Self { api_url: AlivePluginConfig::default_url(), timeout_secs: AlivePluginConfig::default_timeout() }
    }
}

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
    fn kind(&self) -> CommandKind { CommandKind::Simple }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let cfg = PluginConfig::global().get_section::<AlivePluginConfig>("alive");
        info!("[alive] 请求设备状态, url={}, 群={}", cfg.api_url, ctx.group_id);
        let resp = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
            .build()?
            .get(&cfg.api_url)
            .send()
            .await
            .context("请求 alive API 失败")?
            .json::<AliveResponse>()
            .await
            .context("解析 alive API 响应失败")?;

        // 私密模式
        if resp.privacy {
            warn!("[alive] 主人开启了私密模式, 群={}", ctx.group_id);
            return ctx
                .api
                .send_text(ctx.group_id, "主人开启了私密模式哦！不能看！")
                .await;
        }

        info!("[alive] 设备数={}, 群={}", resp.devices.len(), ctx.group_id);

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
