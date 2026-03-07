pub mod fetcher;
pub mod statistics;
pub mod llm;
pub mod renderer;
pub mod screenshot;

use anyhow::Result;
use serde::Deserialize;
use tracing::{info, warn};

use self::fetcher::ChatMessage;

// ── LLM 配置 ──────────────────────────────────────────────────────────────────

/// logic.toml `[smy.llm]` 段。
#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    /// OpenAI 兼容 API 地址
    #[serde(default = "LlmConfig::default_url")]
    pub api_url: String,
    /// API Key
    pub api_key: String,
    /// 模型名称
    #[serde(default = "LlmConfig::default_model")]
    pub model: String,
}

impl LlmConfig {
    fn default_url() -> String { "https://api.deepseek.com/v1".to_string() }
    fn default_model() -> String { "deepseek-chat".to_string() }
}

/// smy 插件配置，从 `logic.toml` 的 `[smy]` 段加载。
/// 所有字段均有默认值，`logic.toml` 不存在时也可正常运行。
#[derive(Debug, Deserialize)]
pub struct SmyPluginConfig {
    /// 截图宽度（像素）
    #[serde(default = "SmyPluginConfig::default_screenshot_width")]
    pub screenshot_width: u32,
    /// LLM 配置（可选，缺少时 -a/--ai 报错提示未配置）
    pub llm: Option<LlmConfig>,
}

impl SmyPluginConfig {
    fn default_screenshot_width() -> u32 { 1200 }
}

impl Default for SmyPluginConfig {
    fn default() -> Self {
        Self {
            screenshot_width: SmyPluginConfig::default_screenshot_width(),
            llm: None,
        }
    }
}

// ── 公共管道 ──────────────────────────────────────────────────────────────────

/// smy 核心管道：统计 → 可选 LLM → 渲染 → 截图，返回 base64 PNG。
///
/// 调用方（命令 / 定时任务）负责消息拉取和图片发送，
/// 本函数只做纯计算 + 截图，不涉及 API 交互。
pub async fn generate_report(
    messages: &[ChatMessage],
    llm_config: Option<&LlmConfig>,
    group_label: &str,
    screenshot_width: u32,
) -> Result<String> {
    // ── 统计分析 ──────────────────────────────────────────────────────────────
    let stats = statistics::analyze(messages);

    // ── LLM 分析（可选） ──────────────────────────────────────────────────────
    let llm_result = if let Some(cfg) = llm_config {
        info!("[smy] 请求 LLM 分析...");
        match llm::analyze(messages, cfg).await {
            Ok(r) => {
                info!("[smy] LLM 分析完成");
                r
            }
            Err(e) => {
                warn!("[smy] LLM 分析失败，使用空结果: {e:#}");
                llm::LlmResult::default()
            }
        }
    } else {
        llm::LlmResult::default()
    };

    // ── 渲染 HTML ─────────────────────────────────────────────────────────────
    let html = renderer::render(&stats, &llm_result, group_label, messages);
    info!("[smy] 渲染完成: HTML {}KB", html.len() / 1024);

    // ── 截图 ──────────────────────────────────────────────────────────────────
    let base64_img = screenshot::capture(&html, screenshot_width).await?;
    info!("[smy] 截图完成: {}KB", base64_img.len() / 1024);

    Ok(base64_img)
}

#[cfg(test)]
mod preview_tests {
	use std::fs;

	use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

	use crate::runtime::pool::MsgKind;

	use super::fetcher::ChatMessage;
	use super::llm::{LlmResult, Quote, Relationship, Topic, UserTitle};
	use super::statistics::Statistics;

	#[tokio::test]
	async fn test_generate_smy_preview_assets() {
		let messages = vec![
			ChatMessage {
				user_id: 10001,
				nickname: "Alice".to_string(),
				time: 1_709_700_000,
				text: "今天把部署脚本重构好了，CI 速度提升明显！".to_string(),

				emoji_count: 1,
				msg_id: 1001,
				kind: MsgKind::Text,
				image_count: 0,
				reply_to: None,
				at_targets: vec![],
				face_ids: vec!["4".to_string()],
			},
			ChatMessage {
				user_id: 10002,
				nickname: "Bob".to_string(),
				time: 1_709_701_000,
				text: "我测了一下，冷启动从 28s 降到 12s，确实很顶。".to_string(),

				emoji_count: 0,
				msg_id: 1002,
				kind: MsgKind::Text,
				image_count: 0,
				reply_to: Some(1001),
				at_targets: vec![10001],
				face_ids: vec![],
			},
			ChatMessage {
				user_id: 10003,
				nickname: "Carol".to_string(),
				time: 1_709_702_000,
				text: "那我们要不要把截图逻辑也加个回归测试？".to_string(),

				emoji_count: 0,
				msg_id: 1003,
				kind: MsgKind::Text,
				image_count: 0,
				reply_to: None,
				at_targets: vec![],
				face_ids: vec![],
			},
			ChatMessage {
				user_id: 10001,
				nickname: "Alice".to_string(),
				time: 1_709_703_000,
				text: "支持，我今晚顺手补上。".to_string(),

				emoji_count: 1,
				msg_id: 1004,
				kind: MsgKind::Text,
				image_count: 0,
				reply_to: None,
				at_targets: vec![],
				face_ids: vec!["4".to_string()],
			},
		];

		let mut hourly = [0u32; 24];
		hourly[9] = 8;
		hourly[10] = 12;
		hourly[11] = 16;
		hourly[21] = 6;

		let stats = Statistics {
			message_count: 412,
			participant_count: 17,
			total_characters: 9365,
			emoji_count: 142,
			image_count: 28,
			most_active_hour: "11:00 - 12:00".to_string(),
			hourly_distribution: hourly,
			top_speakers: vec![
				(10001, "Alice".to_string(), 68),
				(10002, "Bob".to_string(), 59),
				(10003, "Carol".to_string(), 42),
			],
			reply_count: 37,
			at_count: 24,
			top_emoji: Some("4".to_string()),
		};

		let llm = LlmResult {
			topics: vec![
				Topic {
					topic: "部署与性能优化".to_string(),
					contributors: vec!["Alice".to_string(), "Bob".to_string()],
					detail: "@Alice 分享了部署脚本重构结果，@Bob 给出冷启动耗时对比并确认优化有效。".to_string(),
				},
				Topic {
					topic: "测试体系补强".to_string(),
					contributors: vec!["Carol".to_string(), "Alice".to_string()],
					detail: "@Carol 提议补截图回归，@Alice 当场承诺在当晚完成。".to_string(),
				},
			],
			user_titles: vec![
				UserTitle {
					name: "Alice".to_string(),
					title: "效率推进器".to_string(),
					mbti: "ENTJ".to_string(),
					habit: "先做完再同步结论".to_string(),
					reason: "行动快、结果导向，推动讨论迅速落地。".to_string(),
				},
				UserTitle {
					name: "Bob".to_string(),
					title: "数据佐证官".to_string(),
					mbti: "INTP".to_string(),
					habit: "喜欢用实测数据说话".to_string(),
					reason: "每次都能补充可量化指标，帮助团队决策。".to_string(),
				},
			],
			golden_quotes: vec![
				Quote {
					content: "冷启动从 28s 降到 12s，确实很顶。".to_string(),
					sender: "Bob".to_string(),
					reason: "一句话总结优化价值，信息密度高。".to_string(),
				},
			],
			relationships: vec![
				Relationship {
					rel_type: "duo".to_string(),
					members: vec!["Alice".to_string(), "Bob".to_string()],
					label: "跑路搞码组合".to_string(),
					vibe: "一个冲提方案、一个追数据，接龙可谓天衣无缝。".to_string(),
					evidence: "Alice 提重构方案，Bob 立刻补出冷启动耗时对比。".to_string(),
				},
				Relationship {
					rel_type: "group".to_string(),
					members: vec!["Carol".to_string(), "Alice".to_string()],
					label: "质量先锋队".to_string(),
					vibe: "遇到质量问题必集体出动，论完就拿出行动方案。".to_string(),
					evidence: "Carol 提议加回归测试，Alice 当场承诺今晚完成。".to_string(),
				},
			],
		};

		let html = super::renderer::render(&stats, &llm, "smy-preview (mock)", &messages);

		let out_dir = "/tmp/lianbot_smy_preview";
		fs::create_dir_all(out_dir).expect("create out dir should succeed");

		let html_path = format!("{out_dir}/smy_preview.html");
		fs::write(&html_path, &html).expect("write html should succeed");

		let b64 = super::screenshot::capture(&html, 1200)
			.await
			.expect("capture should succeed");
		let png = B64.decode(b64).expect("decode base64 should succeed");
		let png_path = format!("{out_dir}/smy_preview.png");
		fs::write(&png_path, png).expect("write png should succeed");

		eprintln!("SMY 预览 HTML: {html_path}");
		eprintln!("SMY 预览 PNG : {png_path}");
	}
}
