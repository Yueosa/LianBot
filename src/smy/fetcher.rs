use anyhow::{Context, Result};
use serde_json::Value;

use crate::api::ApiClient;

// ── 结构化消息 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub user_id: i64,
    pub nickname: String,
    pub time: i64,
    pub text: String,
    pub has_image: bool,
    pub emoji_count: u32,
}

// ── 时间过滤解析 ──────────────────────────────────────────────────────────────

/// 解析时间字符串如 "30m" / "2h" / "1d"，返回对应的秒数
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "m" => Some(num * 60),
        "h" => Some(num * 3600),
        "d" => Some(num * 86400),
        _ => None,
    }
}

// ── 消息拉取 ──────────────────────────────────────────────────────────────────

/// 从 NapCat 拉取历史消息并结构化
pub async fn fetch(
    api: &ApiClient,
    group_id: i64,
    count: u32,
    time_filter: Option<&str>,
) -> Result<Vec<ChatMessage>> {
    let raw = api
        .get_group_msg_history(group_id, count)
        .await
        .context("拉取群消息历史失败")?;

    let now = chrono::Utc::now().timestamp();
    let cutoff = time_filter
        .and_then(parse_duration)
        .map(|secs| now - secs);

    let mut messages = Vec::with_capacity(raw.len());

    for msg in &raw {
        // 跳过 Bot 自身发的消息
        if msg.get("post_type").and_then(Value::as_str) == Some("message_sent") {
            continue;
        }

        let time = msg.get("time").and_then(Value::as_i64).unwrap_or(0);

        // 时间过滤
        if let Some(cut) = cutoff {
            if time < cut {
                continue;
            }
        }

        let sender = msg.get("sender").cloned().unwrap_or(Value::Null);
        let card = sender.get("card").and_then(Value::as_str).unwrap_or("");
        let nickname = sender.get("nickname").and_then(Value::as_str).unwrap_or("未知");
        let display_name = if card.is_empty() {
            nickname.to_string()
        } else {
            card.to_string()
        };

        let user_id = msg.get("user_id").and_then(Value::as_i64).unwrap_or(0);

        // 解析 message 段
        let segments = msg.get("message").and_then(Value::as_array);
        let (text, has_image, emoji_count) = extract_segments(segments);

        // 跳过空消息
        if text.is_empty() && !has_image {
            continue;
        }

        messages.push(ChatMessage {
            user_id,
            nickname: display_name,
            time,
            text,
            has_image,
            emoji_count,
        });
    }

    Ok(messages)
}

/// 从消息段数组提取文本、是否含图片、表情计数
fn extract_segments(segments: Option<&Vec<Value>>) -> (String, bool, u32) {
    let Some(segs) = segments else {
        return (String::new(), false, 0);
    };

    let mut texts = Vec::new();
    let mut has_image = false;
    let mut emoji_count: u32 = 0;

    for seg in segs {
        let seg_type = seg.get("type").and_then(Value::as_str).unwrap_or("");
        match seg_type {
            "text" => {
                if let Some(t) = seg.get("data").and_then(|d| d.get("text")).and_then(Value::as_str)
                {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        texts.push(trimmed.to_string());
                    }
                }
            }
            "image" => {
                has_image = true;
            }
            "face" | "mface" | "bface" | "sface" => {
                emoji_count += 1;
            }
            _ => {}
        }
    }

    (texts.join(""), has_image, emoji_count)
}

// ── 格式化消息供 LLM 使用 ────────────────────────────────────────────────────

/// 将消息列表格式化为 LLM 可读的纯文本，剔除无关信息
/// 格式: [HH:MM] 昵称: 内容
pub fn format_for_llm(messages: &[ChatMessage]) -> String {
    use chrono::{TimeZone, Utc};

    let mut lines = Vec::with_capacity(messages.len());
    for msg in messages {
        if msg.text.is_empty() {
            continue;
        }
        let dt = Utc.timestamp_opt(msg.time, 0).single();
        let time_str = dt
            .map(|t| {
                // 转为 UTC+8
                let t8 = t + chrono::Duration::hours(8);
                t8.format("%H:%M").to_string()
            })
            .unwrap_or_else(|| "??:??".to_string());

        lines.push(format!("[{}] {}: {}", time_str, msg.nickname, msg.text));
    }
    lines.join("\n")
}
