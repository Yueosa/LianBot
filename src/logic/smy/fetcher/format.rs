use super::model::{ChatMessage, GapLevel, GapWarning};
use crate::runtime::pool::MsgKind;

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
    let mut lines = Vec::with_capacity(messages.len());
    for msg in messages {
        let content = match (&msg.kind, msg.text.is_empty()) {
            (MsgKind::Forward, _) => "[转发消息]".to_owned(),
            (_, true) => continue,
            (_, false) => msg.text.clone(),
        };

        let time_str = crate::runtime::time::from_timestamp(msg.time)
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_else(|| "??:??".to_string());

        let name = if msg.is_bot {
            format!("[Bot]{}", msg.nickname)
        } else {
            msg.nickname.clone()
        };

        lines.push(format!("[{}] {}: {}", time_str, name, content));
    }
    lines.join("\n")
}
