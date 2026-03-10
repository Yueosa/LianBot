use serde_json::Value;

use crate::runtime::pool::{classify_kind, concat_text_segs, PoolMessage};
use crate::runtime::typ::message::MessageSegment;

use super::model::ChatMessage;

/// segment 统计字段（pool_msg_to_chat 和 parse_raw_messages 共用）
pub struct ChatFields {
    pub emoji_count: u32,
    pub image_count: u32,
    pub reply_to: Option<i64>,
    pub at_targets: Vec<i64>,
    pub face_ids: Vec<String>,
}

/// 从 `Vec<MessageSegment>` 提取统计字段
pub fn extract_chat_fields(segments: &[MessageSegment]) -> ChatFields {
    let mut emoji_count: u32 = 0;
    let mut image_count: u32 = 0;
    let mut reply_to: Option<i64> = None;
    let mut at_targets: Vec<i64> = Vec::new();
    let mut face_ids: Vec<String> = Vec::new();

    for seg in segments {
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

    ChatFields { emoji_count, image_count, reply_to, at_targets, face_ids }
}

/// 将 PoolMessage 转为 ChatMessage（smy 统计模块使用）
pub fn pool_msg_to_chat(msg: &PoolMessage) -> ChatMessage {
    let f = extract_chat_fields(&msg.segments);

    ChatMessage {
        user_id:     msg.user_id,
        nickname:    msg.nickname.clone(),
        time:        msg.timestamp,
        text:        msg.text.clone().unwrap_or_default(),
        emoji_count: f.emoji_count,
        msg_id:      msg.msg_id,
        kind:        msg.kind.clone(),
        image_count: f.image_count,
        reply_to:    f.reply_to,
        at_targets:  f.at_targets,
        face_ids:    f.face_ids,
        is_bot:      msg.is_bot,
    }
}

/// 将 NapCat 返回的原始 JSON 消息列表解析为 ChatMessage。
///
/// 统一走 serde → `Vec<MessageSegment>` → `classify_kind` / `concat_text_segs` / `extract_chat_fields`，
/// 与 `PoolMessage::from_api_value` 共享同一套解析逻辑。
pub fn parse_raw_messages(raw: &[Value], cutoff: Option<i64>) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(raw.len());

    for msg in raw {
        let is_bot = msg.get("post_type").and_then(Value::as_str) == Some("message_sent");

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

        // serde 反序列化为 Vec<MessageSegment>，与 pool 路径完全一致
        let segments: Vec<MessageSegment> = msg
            .get("message")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let text = concat_text_segs(&segments).unwrap_or_default();
        let f = extract_chat_fields(&segments);

        // 跳过空消息
        if text.is_empty() && f.image_count == 0 {
            continue;
        }

        let kind = classify_kind(&segments);
        let msg_id = msg.get("message_id").and_then(Value::as_i64).unwrap_or(0);

        messages.push(ChatMessage {
            user_id,
            nickname: display_name,
            time,
            text,
            emoji_count: f.emoji_count,
            msg_id,
            kind,
            image_count: f.image_count,
            reply_to: f.reply_to,
            at_targets: f.at_targets,
            face_ids: f.face_ids,
            is_bot,
        });
    }

    messages
}
