use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};

use crate::command::{Command, CommandContext};
use crate::smy;

/// 默认拉取消息条数
const DEFAULT_COUNT: u32 = 200;
/// 默认时间范围
const DEFAULT_TIME: &str = "12h";

pub struct SmyCommand;

#[async_trait]
impl Command for SmyCommand {
    fn name(&self) -> &str { "smy" }
    fn help(&self) -> &str { "群聊日报 - AI分析群聊并生成分析报告\n  -n / --count <条数>: 拉取消息数量(默认200)\n  -t / --time <时间>: 时间范围(默认12h)\n    支持: 30m / 2h / 1d 等\n\n示例:\n  <smy>\n  <smy> -n 100 -t 6h\n  <日报> -t 1d" }

    fn aliases(&self) -> Vec<&str> {
        vec!["日报"]
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let group_id = ctx.group_id;

        // 检查 LLM 配置
        let llm_config = match &ctx.config.llm {
            Some(c) => c,
            None => {
                return ctx.api.send_text(group_id, "❌ 未配置 LLM，无法生成日报").await;
            }
        };

        // 解析参数：-n 和 -t 独立判断，只有用户显式指定 -t 时才按时间拉取
        let count: u32 = ctx
            .get(&["-n", "--count"])
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_COUNT);

        let time_opt = ctx.get(&["-t", "--time"]);

        let mode_desc = match &time_opt {
            Some(t) => format!("time={t} (时间模式)"),
            None => format!("count={count} (条数模式)"),
        };

        ctx.api
            .send_text(group_id, "📊 正在生成群聊日报，请稍候...")
            .await?;

        info!("[S1] 拉取消息: group={group_id} {mode_desc}");

        // ── S1: 拉取消息 ──────────────────────────────────────────────────────
        let messages = smy::fetcher::fetch(&ctx.api, group_id, count, time_opt).await?;

        if messages.is_empty() {
            return ctx.api.send_text(group_id, "📭 该时间范围内没有聊天记录").await;
        }

        info!("[S1] 拉取完成: {} 条消息", messages.len());

        // ── S2: 统计分析 ──────────────────────────────────────────────────────
        info!("[S2] 统计分析...");
        let stats = smy::statistics::analyze(&messages);

        // ── S3: LLM 分析 ─────────────────────────────────────────────────────
        info!("[S3] 请求 LLM 分析...");
        let llm_result = smy::llm::analyze(&messages, llm_config).await;

        if let Err(ref e) = llm_result {
            warn!("[S3] LLM 分析失败，将使用空结果: {e:#}");
        } else {
            info!("[S3] LLM 分析完成");
        }
        let llm_result = llm_result.unwrap_or_default();

        // ── S4: 渲染 HTML ────────────────────────────────────────────────────
        info!("[S4] 渲染 HTML...");
        let html = smy::renderer::render(&stats, &llm_result, &mode_desc, &messages);
        info!("[S4] HTML 渲染完成: {}KB", html.len() / 1024);

        // ── S5: 截图 ─────────────────────────────────────────────────────────
        info!("[S5] 调用 Chrome 截图...");
        let base64_img = match smy::screenshot::capture(&html).await {
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
