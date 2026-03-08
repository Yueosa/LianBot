use serde_json::Value;

use crate::runtime::pool::{MsgKind, PoolMessage};

use super::model::{ChatMessage, ExtractedSegments};

/// 将 PoolMessage 转为 ChatMessage（smy 统计模块使用）
pub fn pool_msg_to_chat(msg: &PoolMessage) -> ChatMessage {
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
pub fn parse_raw_messages(raw: &[Value], cutoff: Option<i64>) -> Vec<ChatMessage> {
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
