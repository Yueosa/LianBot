use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn};
use std::time::Duration;

use crate::commands::{Command, CommandContext, CommandKind, Dependency, ParamKind, ParamSpec, ValueConstraint};
use crate::logic::config as logic_config;
use crate::logic::smy;
use crate::logic::smy::SmyPluginConfig;
use crate::logic::smy::fetcher::{FetchSource, GapLevel};
use crate::runtime::llm;

pub struct SmyCommand;

#[async_trait]
impl Command for SmyCommand {
    fn name(&self) -> &str { "smy" }
    fn help(&self) -> &str { "群聊总结：统计分析 + 可选 AI 总结" }
    fn aliases(&self) -> &[&str] { &["日报"] }
    fn kind(&self) -> CommandKind { CommandKind::Advanced }
    fn declared_params(&self) -> &[ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec { keys: &["-a", "--ai"],    kind: ParamKind::Flag,                                                         required: false, help: "开启 AI 文字总结" },
            ParamSpec { keys: &["-t", "--time"],  kind: ParamKind::Value(ValueConstraint::Any),                                  required: false, help: "时间范围，如 30m / 2h / 1d（默认 1d）" },
        ];
        PARAMS
    }
    fn dependencies(&self) -> &[Dependency] { &[Dependency::Config] }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let group_id = match ctx.group_id() {
            Some(gid) => gid,
            None => return ctx.reply("❌ 群聊总结仅在群聊中可用").await,
        };
        let cfg = logic_config::section::<SmyPluginConfig>("smy");

        // AI 总结开关：默认关闭，用户显式传 -a/--ai 才启用
        let with_ai = ["-a", "--ai"].iter().any(|k| ctx.params.contains_key(*k));

        // 仅在启用 AI 时检查 LLM 配置
        if with_ai && llm::get().is_none() {
            return ctx.reply("❌ 未配置 LLM，无法进行 AI 总结（可去掉 -a 使用纯统计模式）").await;
        }

        let time_opt = ctx.get(&["-t", "--time"]);

        let time_window_secs = match time_opt {
            Some(v) => match smy::fetcher::parse_duration(v) {
                Some(secs) => secs,
                None => return ctx.reply("❌ 时间格式错误，请使用 30m / 2h / 1d").await,
            },
            None => 86400,
        };
        let mode_desc = format!("time={} (时间模式)", time_opt.unwrap_or("1d"));

        info!("[smy] 模式: {mode_desc}, ai={with_ai}");

        ctx.reply("📊 正在总结，请稍候...").await?;

        let fetch_result = smy::fetcher::fetch(
            &ctx.api,
            &ctx.pool,
            group_id,
            Duration::from_secs(time_window_secs as u64),
        ).await?;
        let messages = fetch_result.messages;

        if matches!(fetch_result.source, FetchSource::ApiExhausted) {
            ctx.reply("⚠️ 服务端历史消息不足，当前时间窗口未被完整覆盖").await?;
        }

        if let Some(gap) = fetch_result.gap {
            let level = match gap.level {
                GapLevel::Day => "跨天",
                GapLevel::Week => "跨周",
                GapLevel::Month => "跨月",
            };
            ctx.reply(
                &format!(
                    "⚠️ 检测到消息时间断层（{}，约 {:.1} 小时），统计结果可能不连续",
                    level, gap.gap_hours
                ),
            ).await?;
        }

        if messages.is_empty() {
            return ctx.reply("📭 该时间范围内没有聊天记录").await;
        }

        debug!("[smy] 拉取完成: {} 条消息", messages.len());

        // ── 核心管道：统计 → LLM → 渲染 → 截图 ──────────────────────────────
        let base64_img = match smy::generate_report(
            &messages,
            with_ai,
            &mode_desc,
            cfg.screenshot_width,
        ).await {
            Ok(b) => b,
            Err(e) => {
                warn!("[smy] 管道失败: {e:#}");
                return ctx.reply(&format!("❌ 生成报告失败: {e}")).await;
            }
        };

        // ── 发送图片 ─────────────────────────────────────────────────────────
        ctx.reply_image(&format!("base64://{base64_img}")).await?;
        Ok(())
    }
}
