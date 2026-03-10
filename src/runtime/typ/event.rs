// 这个文件为 OneBot v11 协议类型层，部分字段/方法暂未使用但保留供未来扩展。
#![allow(dead_code)]

use serde::Deserialize;
use super::message::MessageSegment;

// ── 顶层事件枚举 ──────────────────────────────────────────────────────────────
//
// OneBot v11 的 post_type 字段决定事件类型：
//   message | notice | request | meta_event
//
// 使用 serde 内部 tag 派发；未知类型被 Unknown 吸收，不会 panic

#[derive(Debug, Deserialize)]
#[serde(tag = "post_type", rename_all = "snake_case")]
pub enum OneBotEvent {
    Message(MessageEvent),
    /// Bot 自身发送的消息（NapCat 上报 post_type = "message_sent"）
    MessageSent(MessageEvent),
    Notice(NoticeEvent),
    Request(RequestEvent),
    MetaEvent(MetaEvent),

    /// 兜底：任何未知 post_type 都落这里，保证反序列化不失败
    #[serde(other)]
    Unknown,
}

// ── 消息事件 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct MessageEvent {
    /// "group" | "private"
    pub message_type: MessageType,

    /// 消息 ID
    pub message_id: Option<i64>,

    /// 发送者 QQ 号
    pub user_id: i64,

    /// 群号（私聊时为 None）
    pub group_id: Option<i64>,

    /// 结构化消息段列表
    pub message: Vec<MessageSegment>,

    /// 原始消息文本（CQ 码格式）
    pub raw_message: Option<String>,

    /// 发送者信息
    pub sender: Option<Sender>,

    /// 时间戳
    pub time: Option<i64>,
}

impl MessageEvent {
    /// 提取第一个文本段的内容（常用于命令解析）
    pub fn first_text(&self) -> Option<&str> {
        self.message
            .iter()
            .find_map(|seg| seg.as_text())
    }

    /// 拼接所有文本段（用于关键词匹配 / AI 对话）
    pub fn full_text(&self) -> String {
        self.message
            .iter()
            .filter_map(|seg| seg.as_text())
            .collect::<Vec<_>>()
            .join("")
    }

    /// 生成人类可读的消息摘要（用于日志），所有段类型均可见。
    ///
    /// 示例：`"[@123] 你好 [图片]"`、`"[表情] [表情]"`
    pub fn describe(&self) -> String {
        let mut buf = String::new();
        for seg in &self.message {
            match seg.seg_type.as_str() {
                "text"  => if let Some(t) = seg.as_text() { buf.push_str(t); },
                "image" => buf.push_str("[图片]"),
                "face" | "mface" | "bface" | "sface" => buf.push_str("[表情]"),
                "at"    => {
                    let qq = seg.data.get("qq").map(|v| {
                        if let Some(s) = v.as_str() { s.to_string() }
                        else if let Some(n) = v.as_i64() { n.to_string() }
                        else { "?".to_string() }
                    }).unwrap_or_else(|| "?".to_string());
                    buf.push_str(&format!("[@{qq}]"));
                }
                "reply"   => buf.push_str("[回复]"),
                "forward" => buf.push_str("[转发]"),
                "record"  => buf.push_str("[语音]"),
                "video"   => buf.push_str("[视频]"),
                "file"    => buf.push_str("[文件]"),
                other     => { buf.push('['); buf.push_str(other); buf.push(']'); }
            }
        }
        buf
    }

    /// 判断是否群消息
    pub fn is_group(&self) -> bool {
        matches!(self.message_type, MessageType::Group)
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Group,
    Private,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Sender {
    pub user_id: Option<i64>,
    pub nickname: Option<String>,
    pub card: Option<String>,     // 群名片
    pub role: Option<String>,     // "owner" | "admin" | "member"
}

// ── 通知事件（预留） ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct NoticeEvent {
    pub notice_type: Option<String>,
    pub group_id: Option<i64>,
    pub user_id: Option<i64>,
    // 其余字段按需扩展
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

// ── 请求事件（预留） ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct RequestEvent {
    pub request_type: Option<String>,
    pub group_id: Option<i64>,
    pub user_id: Option<i64>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

// ── 元事件（心跳等） ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct MetaEvent {
    pub meta_event_type: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}
