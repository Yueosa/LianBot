use super::model::{ChatMessage, GapLevel, GapWarning};

pub fn detect_gap(messages: &[ChatMessage]) -> Option<GapWarning> {
    if messages.len() < 2 {
        return None;
    }

    let mut max_gap_secs = 0i64;
    let mut gap_start = 0i64;
    let mut gap_end = 0i64;

    for pair in messages.windows(2) {
        let left = pair[0].time;
        let right = pair[1].time;
        if right <= left {
            continue;
        }
        let gap = right - left;
        if gap > max_gap_secs {
            max_gap_secs = gap;
            gap_start = left;
            gap_end = right;
        }
    }

    let level = if max_gap_secs >= 30 * 24 * 3600 {
        Some(GapLevel::Month)
    } else if max_gap_secs >= 7 * 24 * 3600 {
        Some(GapLevel::Week)
    } else if max_gap_secs >= 24 * 3600 {
        Some(GapLevel::Day)
    } else {
        None
    }?;

    Some(GapWarning {
        level,
        gap_hours: max_gap_secs as f64 / 3600.0,
        gap_start,
        gap_end,
    })
}

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
