use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::commands::{Command, CommandContext, CommandKind, http_client};
use crate::logic::config as logic_config;

// ── 插件配置 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AlivePluginConfig {
    /// Alive API 地址（必填，空则跳过）
    #[serde(default)]
    api_url: String,
    /// 请求超时（秒），默认 5
    #[serde(default = "AlivePluginConfig::default_timeout")]
    timeout_secs: u64,
}

impl AlivePluginConfig {
    fn default_timeout() -> u64 { 5 }
}

impl Default for AlivePluginConfig {
    fn default() -> Self {
        Self { api_url: String::new(), timeout_secs: 5 }
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
    #[serde(rename = "status-hint", default)]
    status_hint: String,
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
    fn name(&self) -> &str { "alive" }
    fn help(&self) -> &str { "查看主人当前的设备在线状态" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }
    fn tool_description(&self) -> Option<&str> {
        Some("查询主人各设备的在线状态和正在使用的软件名称。返回一列设备名及其当前运行的应用，适合想知道主人在用什么软件、是否在线时调用")
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let cfg = logic_config::section::<AlivePluginConfig>("alive");
        if cfg.api_url.is_empty() {
            return ctx.reply("❌ alive 未配置 api_url，请在 logic.toml [alive] 中设置").await;
        }
        debug!("[alive] 请求设备状态, url={}", cfg.api_url);
        let resp = http_client()
            .get(&cfg.api_url)
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
            .send()
            .await
            .context("请求 alive API 失败")?
            .json::<AliveResponse>()
            .await
            .context("解析 alive API 响应失败")?;

        // 私密模式
        if resp.privacy {
            warn!("[alive] 主人开启了私密模式");
            return ctx
                .reply("主人开启了私密模式哦！不能看！")
                .await;
        }

        debug!("[alive] 设备数={}", resp.devices.len());

        let mut lines: Vec<String> = Vec::new();

        // 标题 + 状态提示
        lines.push(resp.site.name.clone());
        if !resp.site.status_hint.is_empty() {
            lines.push(resp.site.status_hint.clone());
        }
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

        ctx.reply(&lines.join("\n")).await
    }
}
