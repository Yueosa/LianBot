use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};
use std::time::Duration;

use crate::commands::{Command, CommandContext, CommandKind, Dependency, ParamKind, ParamSpec, ValueConstraint};
use crate::runtime::logic_config;
use crate::logic::smy;
use crate::logic::smy::SmyPluginConfig;
use crate::logic::smy::fetcher::{FetchSource, GapLevel};

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
        let group_id = ctx.group_id;
        let cfg = logic_config::section::<SmyPluginConfig>("smy");

        // AI 总结开关：默认关闭，用户显式传 -a/--ai 才启用
        let with_ai = ["-a", "--ai"].iter().any(|k| ctx.params.contains_key(*k));

        // 仅在启用 AI 时检查 LLM 配置
        let llm_config = if with_ai {
            match &cfg.llm {
                Some(c) => Some(c.clone()),
                None => {
                    return ctx.api.send_text(group_id, "❌ 未配置 LLM，无法进行 AI 总结（可去掉 -a 使用纯统计模式）").await;
                }
            }
        } else {
            None
        };

        let time_opt = ctx.get(&["-t", "--time"]);

        let time_window_secs = match time_opt {
            Some(v) => match smy::fetcher::parse_duration(v) {
                Some(secs) => secs,
                None => return ctx.api.send_text(group_id, "❌ 时间格式错误，请使用 30m / 2h / 1d").await,
            },
            None => 86400,
        };
        let mode_desc = format!("time={} (时间模式)", time_opt.unwrap_or("1d"));

        info!("[S0] 模式: group={group_id} {}, ai={with_ai}", mode_desc);

        ctx.api
            .send_text(group_id, "📊 正在总结，请稍候...")
            .await?;

        info!("[S1] 拉取消息: group={group_id} {mode_desc}");

        // ── S1: 拉取消息 ──────────────────────────────────────────────────────
        let fetch_result = smy::fetcher::fetch(
            &ctx.api,
            &ctx.pool,
            group_id,
            Duration::from_secs(time_window_secs as u64),
        ).await?;
        let messages = fetch_result.messages;

        if matches!(fetch_result.source, FetchSource::ApiExhausted) {
            ctx.api
                .send_text(group_id, "⚠️ 服务端历史消息不足，当前时间窗口未被完整覆盖")
                .await?;
        }

        if let Some(gap) = fetch_result.gap {
            let level = match gap.level {
                GapLevel::Day => "跨天",
                GapLevel::Week => "跨周",
                GapLevel::Month => "跨月",
            };
            ctx.api
                .send_text(
                    group_id,
                    &format!(
                        "⚠️ 检测到消息时间断层（{}，约 {:.1} 小时），统计结果可能不连续",
                        level, gap.gap_hours
                    ),
                )
                .await?;
        }

        if messages.is_empty() {
            return ctx.api.send_text(group_id, "📭 该时间范围内没有聊天记录").await;
        }

        info!("[S1] 拉取完成: {} 条消息", messages.len());

        // ── S2: 统计分析 ──────────────────────────────────────────────────────
        info!("[S2] 统计分析...");
        let stats = smy::statistics::analyze(&messages);

        // ── S3: LLM 分析（可选） ──────────────────────────────────────────────
        let llm_result = if let Some(config) = llm_config {
            info!("[S3] 请求 LLM 分析...");
            let llm_result = smy::llm::analyze(&messages, &config).await;

            if let Err(ref e) = llm_result {
                warn!("[S3] LLM 分析失败，将使用空结果: {e:#}");
            } else {
                info!("[S3] LLM 分析完成");
            }
            llm_result.unwrap_or_default()
        } else {
            info!("[S3] 未启用 AI，总结步骤跳过（仅统计模式）");
            smy::llm::LlmResult::default()
        };

        // ── S4: 渲染 HTML ────────────────────────────────────────────────────
        info!("[S4] 渲染 HTML...");
        let html = smy::renderer::render(&stats, &llm_result, &mode_desc, &messages);
        info!("[S4] HTML 渲染完成: {}KB", html.len() / 1024);

        // ── S5: 截图 ─────────────────────────────────────────────────────────
        info!("[S5] 调用 Chrome 截图...");
        let base64_img = match smy::screenshot::capture(&html, cfg.screenshot_width).await {
            Ok(b) => {
                info!("[S5] 截图完成: {}KB", b.len() / 1024);
                b
            }
            Err(e) => {
                warn!("[S5] 截图失败: {e:#}");
                return ctx.api.send_text(group_id, &format!("❌ 截图失败: {e}")).await;
            }
        };

        // ── S6: 发送图片 ─────────────────────────────────────────────────────
        info!("[S6] 发送图片...");
        let file = format!("base64://{base64_img}");
        ctx.api.send_image(group_id, &file).await?;

        info!("[S6] 群聊日报发送完成: group={group_id}");
        Ok(())
    }
}
