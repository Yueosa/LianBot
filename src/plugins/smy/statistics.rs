use std::collections::{HashMap, HashSet};

use super::fetcher::ChatMessage;

// ── 统计结果 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Statistics {
    pub message_count: usize,
    pub participant_count: usize,
    pub total_characters: usize,
    pub emoji_count: u32,
    pub image_count: usize,
    pub most_active_hour: String,
    pub hourly_distribution: [u32; 24],
    /// 发言数 top 榜: (昵称, 条数)
    pub top_speakers: Vec<(String, usize)>,
}

// ── 统计逻辑 ──────────────────────────────────────────────────────────────────

pub fn analyze(messages: &[ChatMessage]) -> Statistics {
    let mut participants: HashSet<i64> = HashSet::new();
    let mut total_chars: usize = 0;
    let mut emoji_count: u32 = 0;
    let mut image_count: usize = 0;
    let mut hourly: [u32; 24] = [0; 24];
    let mut speaker_count: HashMap<i64, (String, usize)> = HashMap::new();

    for msg in messages {
        participants.insert(msg.user_id);
        total_chars += msg.text.chars().count();
        emoji_count += msg.emoji_count;
        if msg.has_image {
            image_count += 1;
        }

        // 按小时分桶 (UTC+8)
        let hour = ((msg.time % 86400 + 8 * 3600) % 86400) / 3600;
        hourly[hour as usize] += 1;

        // 发言人统计
        speaker_count
            .entry(msg.user_id)
            .and_modify(|(_, c)| *c += 1)
            .or_insert_with(|| (msg.nickname.clone(), 1));
    }

    // 最活跃时段
    let peak_hour = hourly.iter().enumerate().max_by_key(|&(_, c)| c).map(|(h, _)| h).unwrap_or(0);
    let most_active_hour = format!("{:02}:00 - {:02}:00", peak_hour, (peak_hour + 1) % 24);

    // Top speakers (按发言数降序)
    let mut top_speakers: Vec<(String, usize)> = speaker_count
        .into_values()
        .collect();
    top_speakers.sort_by(|a, b| b.1.cmp(&a.1));
    top_speakers.truncate(10);

    Statistics {
        message_count: messages.len(),
        participant_count: participants.len(),
        total_characters: total_chars,
        emoji_count,
        image_count,
        most_active_hour,
        hourly_distribution: hourly,
        top_speakers,
    }
}
