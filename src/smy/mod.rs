pub mod fetcher;
pub mod statistics;
pub mod llm;
pub mod renderer;
pub mod screenshot;

#[cfg(test)]
mod preview_tests {
	use std::fs;

	use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

	use super::fetcher::ChatMessage;
	use super::llm::{LlmResult, Quote, Topic, UserTitle};
	use super::statistics::Statistics;

	#[tokio::test]
	async fn test_generate_smy_preview_assets() {
		let messages = vec![
			ChatMessage {
				user_id: 10001,
				nickname: "Alice".to_string(),
				time: 1_709_700_000,
				text: "今天把部署脚本重构好了，CI 速度提升明显！".to_string(),
				has_image: false,
				emoji_count: 1,
			},
			ChatMessage {
				user_id: 10002,
				nickname: "Bob".to_string(),
				time: 1_709_701_000,
				text: "我测了一下，冷启动从 28s 降到 12s，确实很顶。".to_string(),
				has_image: false,
				emoji_count: 0,
			},
			ChatMessage {
				user_id: 10003,
				nickname: "Carol".to_string(),
				time: 1_709_702_000,
				text: "那我们要不要把截图逻辑也加个回归测试？".to_string(),
				has_image: false,
				emoji_count: 0,
			},
			ChatMessage {
				user_id: 10001,
				nickname: "Alice".to_string(),
				time: 1_709_703_000,
				text: "支持，我今晚顺手补上。".to_string(),
				has_image: false,
				emoji_count: 1,
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
				("Alice".to_string(), 68),
				("Bob".to_string(), 59),
				("Carol".to_string(), 42),
			],
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
		};

		let html = super::renderer::render(&stats, &llm, "smy-preview (mock)", &messages);

		let out_dir = "/tmp/lianbot_smy_preview";
		fs::create_dir_all(out_dir).expect("create out dir should succeed");

		let html_path = format!("{out_dir}/smy_preview.html");
		fs::write(&html_path, &html).expect("write html should succeed");

		let b64 = super::screenshot::capture(&html)
			.await
			.expect("capture should succeed");
		let png = B64.decode(b64).expect("decode base64 should succeed");
		let png_path = format!("{out_dir}/smy_preview.png");
		fs::write(&png_path, png).expect("write png should succeed");

		eprintln!("SMY 预览 HTML: {html_path}");
		eprintln!("SMY 预览 PNG : {png_path}");
	}
}
