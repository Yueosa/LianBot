use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{info, warn};

use crate::runtime::{
    api::ApiClient,
    pool::{MessagePool, MsgKind, Pool, PoolMessage},
};

// ── 结构化消息 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub user_id: i64,
    pub nickname: String,
    pub time: i64,
    pub text: String,
    pub emoji_count: u32,
    // 扩展字段——从 PoolMessage/原始 JSON 中直接提取
    /// 消息 ID（供互动图谱、回复链分析等未来功能使用）
    #[allow(dead_code)]
    pub msg_id: i64,
    /// 消息类型（供消息类型分布图使用）
    #[allow(dead_code)]
    pub kind: MsgKind,
    pub image_count: u32,
    pub reply_to: Option<i64>,
    pub at_targets: Vec<i64>,
    pub face_ids: Vec<String>,
}

/// 从 message 段数组提取的结构化结果
struct ExtractedSegments {
    text: String,
    emoji_count: u32,
    image_count: u32,
    reply_to: Option<i64>,
    at_targets: Vec<i64>,
    face_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSource {
    Pool,
    Api,
    ApiExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapLevel {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone)]
pub struct GapWarning {
    pub level: GapLevel,
    pub gap_hours: f64,
    pub gap_start: i64,
    pub gap_end: i64,
}

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub messages: Vec<ChatMessage>,
    pub gap: Option<GapWarning>,
    pub source: FetchSource,
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

/// 从消息池或 NapCat 拉取历史消息并结构化。
///
/// 读取策略（time-first）：
///   1. 优先从内存池读取（微秒级，无网络消耗）
///   2. pool 未完整覆盖 → 分页调用 NapCat API（count=5000, message_seq 回溯）
///   3. API 结果自动 back-seed 到 pool（下次直接命中）
pub async fn fetch(
    api: &ApiClient,
    pool: &Option<Arc<Pool>>,
    group_id: i64,
    time_window: Duration,
) -> Result<FetchResult> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - time_window.as_secs() as i64;

    // 优先尝试 pool 路径
    if let Some(pool) = pool {
        let pool_msgs = pool.range(group_id, cutoff, now).await;
        let oldest = pool.oldest_timestamp(group_id).await;
        if !pool_msgs.is_empty() && oldest.is_some_and(|t| t <= cutoff) {
            info!(
                "[fetcher] 时间模式: pool完整覆盖 {} 条 (oldest={}, cutoff={})",
                pool_msgs.len(),
                oldest.unwrap(),
                cutoff
            );
            let mut messages: Vec<ChatMessage> = pool_msgs.iter().map(pool_msg_to_chat).collect();
            messages.sort_by_key(|m| m.time);
            return Ok(FetchResult {
                gap: detect_gap(&messages),
                messages,
                source: FetchSource::Pool,
            });
        }
        info!(
            "[fetcher] 时间模式: pool起点={:?} > cutoff={}, 回退 API 分页",
            oldest, cutoff
        );
    } else {
        info!("[fetcher] 无消息池，直接走 API 分页");
    }

    let (raw, reached_cutoff, earliest_ts) = fetch_api_until_cutoff(api, group_id, cutoff).await?;
    if let Some(pool) = pool {
        back_seed_pool(pool, &raw, group_id, cutoff).await;
    }
    let mut messages = parse_raw_messages(&raw, Some(cutoff));
    messages.sort_by_key(|m| m.time);

    let source = if reached_cutoff {
        FetchSource::Api
    } else {
        warn!(
            "[fetcher] 服务端历史已穷尽但未覆盖请求窗口: earliest={:?}, cutoff={}, group={}",
            earliest_ts, cutoff, group_id
        );
        FetchSource::ApiExhausted
    };

    info!(
        "[fetcher] 时间模式: API过滤后 {} 条, source={:?}",
        messages.len(), source
    );

    Ok(FetchResult {
        gap: detect_gap(&messages),
        messages,
        source,
    })
}

/// 将 pool 中的 API 原始 JSON 批量写入 pool（back-seeding）
async fn back_seed_pool(pool: &Pool, raw: &[Value], group_id: i64, cutoff: i64) {
    for value in raw {
        let ts = value.get("time").and_then(Value::as_i64).unwrap_or(0);
        if ts < cutoff {
            continue;
        }
        if let Some(msg) = PoolMessage::from_api_value(value, group_id) {
            pool.push(msg).await;
        }
    }
}

async fn fetch_api_until_cutoff(
    api: &ApiClient,
    group_id: i64,
    cutoff: i64,
) -> Result<(Vec<Value>, bool, Option<i64>)> {
    let mut all = Vec::<Value>::new();
    let mut seen_ids = std::collections::HashSet::<i64>::new();
    let mut page_seq: Option<i64> = None;
    let mut reached_cutoff = false;
    let mut earliest_ts: Option<i64> = None;

    for _ in 0..50 {
        let page = api
            .get_group_msg_history_paged(group_id, 5000, page_seq)
            .await
            .context("分页拉取群消息历史失败")?;

        if page.is_empty() {
            break;
        }

        let page_earliest = page.first().and_then(|m| m.get("time")).and_then(Value::as_i64);
        if let Some(ts) = page_earliest {
            earliest_ts = Some(earliest_ts.map_or(ts, |old| old.min(ts)));
            if ts <= cutoff {
                reached_cutoff = true;
            }
        }

        for msg in &page {
            let msg_id = msg.get("message_id").and_then(Value::as_i64).unwrap_or(0);
            if msg_id != 0 {
                if seen_ids.insert(msg_id) {
                    all.push(msg.clone());
                }
            } else {
                all.push(msg.clone());
            }
        }

        if reached_cutoff || page.len() < 5000 {
            break;
        }

        let next_seq = page
            .first()
            .and_then(|m| m.get("message_seq").and_then(Value::as_i64))
            .or_else(|| page.first().and_then(|m| m.get("message_id").and_then(Value::as_i64)));

        if next_seq.is_none() || next_seq == page_seq {
            break;
        }
        page_seq = next_seq;
    }

    Ok((all, reached_cutoff, earliest_ts))
}

fn detect_gap(messages: &[ChatMessage]) -> Option<GapWarning> {
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

/// 将 PoolMessage 转为 ChatMessage（smy 统计模块使用）
fn pool_msg_to_chat(msg: &PoolMessage) -> ChatMessage {
    let mut emoji_count: u32 = 0;
    let mut image_count: u32 = 0;
    let mut reply_to: Option<i64> = None;
    let mut at_targets: Vec<i64> = Vec::new();
    let mut face_ids: Vec<String> = Vec::new();

    for seg in &msg.segments {
        if seg.is_face() {
            emoji_count += 1;
            if let Some(id) = seg.face_id() {
                face_ids.push(id);
            }
        } else if seg.is_image() {
            image_count += 1;
        } else if seg.is_reply() {
            if reply_to.is_none() {
                reply_to = seg.reply_id();
            }
        } else if seg.is_at() {
            if let Some(qq) = seg.at_qq_id() {
                at_targets.push(qq);
            }
        }
    }

    ChatMessage {
        user_id:     msg.user_id,
        nickname:    msg.nickname.clone(),
        time:        msg.timestamp,
        text:        msg.text.clone().unwrap_or_default(),
        emoji_count,
        msg_id:      msg.msg_id,
        kind:        msg.kind.clone(),
        image_count,
        reply_to,
        at_targets,
        face_ids,
    }
}

/// 将 NapCat 返回的原始 JSON 消息列表解析为 ChatMessage
fn parse_raw_messages(raw: &[serde_json::Value], cutoff: Option<i64>) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(raw.len());

    for msg in raw {
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
        let extracted = extract_segments(segments);
        let (text, emoji_count) = (&extracted.text, extracted.emoji_count);

        // 跳过空消息
        if text.is_empty() && extracted.image_count == 0 {
            continue;
        }

        // 推导 MsgKind
        let kind = if extracted.image_count > 0 && !text.is_empty() {
            MsgKind::Mixed
        } else if extracted.image_count > 0 {
            MsgKind::Image
        } else if extracted.reply_to.is_some() {
            MsgKind::Reply
        } else {
            MsgKind::Text
        };

        let msg_id = msg.get("message_id").and_then(Value::as_i64).unwrap_or(0);

        messages.push(ChatMessage {
            user_id,
            nickname: display_name,
            time,
            text: extracted.text,
            emoji_count,
            msg_id,
            kind,
            image_count: extracted.image_count,
            reply_to: extracted.reply_to,
            at_targets: extracted.at_targets,
            face_ids: extracted.face_ids,
        });
    }

    messages
}

/// 从消息段数组提取文本、是否含图片、表情计数及其他结构化字段
fn extract_segments(segments: Option<&Vec<Value>>) -> ExtractedSegments {
    let Some(segs) = segments else {
        return ExtractedSegments {
            text: String::new(),
            emoji_count: 0,
            image_count: 0,
            reply_to: None,
            at_targets: Vec::new(),
            face_ids: Vec::new(),
        };
    };

    let mut texts = Vec::new();
    let mut emoji_count: u32 = 0;
    let mut image_count: u32 = 0;
    let mut reply_to: Option<i64> = None;
    let mut at_targets: Vec<i64> = Vec::new();
    let mut face_ids: Vec<String> = Vec::new();

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
                image_count += 1;
            }
            "face" | "mface" | "bface" | "sface" => {
                emoji_count += 1;
                let data = seg.get("data");
                if let Some(id) = data.and_then(|d| d.get("id")).and_then(Value::as_str) {
                    face_ids.push(id.to_string());
                } else if let Some(id) = data.and_then(|d| d.get("id")).and_then(Value::as_i64) {
                    face_ids.push(id.to_string());
                }
            }
            "reply" => {
                if reply_to.is_none() {
                    reply_to = seg.get("data").and_then(|d| d.get("id")).and_then(Value::as_i64);
                }
            }
            "at" => {
                if let Some(data) = seg.get("data") {
                    if let Some(qq) = data.get("qq").and_then(Value::as_i64) {
                        at_targets.push(qq);
                    } else if let Some(qq_str) = data.get("qq").and_then(Value::as_str) {
                        if let Ok(qq) = qq_str.parse::<i64>() {
                            at_targets.push(qq);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    ExtractedSegments {
        text: texts.join(""),
        emoji_count,
        image_count,
        reply_to,
        at_targets,
        face_ids,
    }
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
