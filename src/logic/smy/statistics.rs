use std::collections::{HashMap, HashSet};

use super::fetcher::ChatMessage;

// ── 统计结果 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Statistics {
    pub message_count: usize,
    pub participant_count: usize,
    pub total_characters: usize,
    pub emoji_count: u32,
    /// 图片消息数量（供消息类型分布图使用，当前渲染层暂未展示）
    #[allow(dead_code)]
    pub image_count: usize,
    pub most_active_hour: String,
    pub hourly_distribution: [u32; 24],
    /// 发言数 top 榜: (user_id, 昵称, 条数)
    pub top_speakers: Vec<(i64, String, usize)>,
    pub reply_count: usize,
    pub at_count: usize,
    pub top_emoji: Option<String>,
}

// ── 统计逻辑 ──────────────────────────────────────────────────────────────────

pub fn analyze(messages: &[ChatMessage]) -> Statistics {
    let mut participants: HashSet<i64> = HashSet::new();
    let mut total_chars: usize = 0;
    let mut emoji_count: u32 = 0;
    let mut image_count: usize = 0;
    let mut reply_count: usize = 0;
    let mut at_count: usize = 0;
    let mut hourly: [u32; 24] = [0; 24];
    let mut speaker_count: HashMap<i64, (String, usize)> = HashMap::new();
    let mut face_freq: HashMap<String, usize> = HashMap::new();

    for msg in messages {
        participants.insert(msg.user_id);
        total_chars += msg.text.chars().count();
        emoji_count += msg.emoji_count;
        image_count += msg.image_count as usize;

        if msg.reply_to.is_some() {
            reply_count += 1;
        }
        at_count += msg.at_targets.len();

        for fid in &msg.face_ids {
            *face_freq.entry(fid.clone()).or_insert(0) += 1;
        }

        let hour = crate::runtime::time::hour_of_day(msg.time);
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

    // Top 10 speakers (按发言数降序): (user_id, nickname, count)
    let mut top_speakers: Vec<(i64, String, usize)> = speaker_count
        .into_iter()
        .map(|(uid, (name, cnt))| (uid, name, cnt))
        .collect();
    top_speakers.sort_by(|a, b| b.2.cmp(&a.2));
    top_speakers.truncate(10);

    // 最热表情 face_id
    let top_emoji = face_freq
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(id, _)| id);

    Statistics {
        message_count: messages.len(),
        participant_count: participants.len(),
        total_characters: total_chars,
        emoji_count,
        image_count,
        most_active_hour,
        hourly_distribution: hourly,
        top_speakers,
        reply_count,
        at_count,
        top_emoji,
    }
}
