use serde::Deserialize;
use serde_json::Value;

use crate::runtime::permission::Scope;
use crate::runtime::typ::event::{MessageEvent, Sender};
use crate::runtime::typ::message::MessageSegment;

// ── 池配置 ────────────────────────────────────────────────────────────────────

/// runtime.toml `[pool]` 段。
#[derive(Debug, Deserialize)]
pub struct PoolConfig {
    /// 每个群的内存缓冲最大消息条数，默认 3000
    #[serde(default = "PoolConfig::default_capacity")]
    pub per_group_capacity: usize,
    /// 内存淘汰阈值（秒），超过此时间的消息被清理，默认 25h（比 smy 的 24h 窗口多 1h 余量）
    #[serde(default = "PoolConfig::default_evict")]
    pub evict_after_secs: i64,
}

impl PoolConfig {
    fn default_capacity() -> usize { 3000 }
    fn default_evict() -> i64 { 90000 }
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            per_group_capacity: Self::default_capacity(),
            evict_after_secs:   Self::default_evict(),
        }
    }
}

// ── PoolMessage 及相关类型 ────────────────────────────────────────────────────

/// 消息池中存储的统一消息结构。
/// 不存储 NapCat raw 字段，精简内存占用。
#[derive(Debug, Clone)]
pub struct PoolMessage {
    /// OneBot message_id（接近 i32::MAX，使用 i64）
    pub msg_id: i64,
    pub scope: Scope,
    #[allow(dead_code)]
    pub user_id: i64,
    /// sender.card（群名片）|| sender.nickname，已提取
    #[allow(dead_code)]
    pub nickname: String,
    /// 秒级 Unix 时间戳
    pub timestamp: i64,
    #[allow(dead_code)]
    pub kind: MsgKind,
    /// 所有 text segment 拼接，无文字则 None
    #[allow(dead_code)]
    pub text: Option<String>,
    /// 消息段——直接复用 typ::MessageSegment，不重新定义
    #[allow(dead_code)]
    pub segments: Vec<MessageSegment>,
    /// 是否为 Bot 自身发送的消息
    #[allow(dead_code)]
    pub is_bot: bool,
    /// 处理状态（新消息默认 Pending，dispatcher 在命令执行后更新）
    pub status: MsgStatus,
    /// 命令处理记录（仅当 status != Pending 时有值）
    pub process: Option<ProcessRecord>,
}

/// 消息类型分类
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgKind {
    Text,   // 纯文本
    Image,  // 含图片 segment
    Face,   // 纯 QQ 表情
    Reply,  // 含回复引用
    At,     // 含 @某人（无 reply）
    Card,   // json segment（分享卡片）
    File,   // file segment（群文件）
    Forward, // 合并转发
    Mixed,  // 多类型混合（如 text + image）
    Other,  // 未分类
}

/// 消息处理状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgStatus {
    /// 已入池，未被任何命令处理
    Pending,
    /// 命令执行成功
    Done,
    /// 命令执行失败
    #[allow(dead_code)]
    Error,
}

/// 命令处理记录（dispatcher 在 cmd.execute() 之后填充）
#[derive(Debug, Clone)]
pub struct ProcessRecord {
    /// 哪个命令处理的（纯名字，如 "acg"）
    #[allow(dead_code)]
    pub command: String,
    /// 执行耗时（毫秒）
    #[allow(dead_code)]
    pub duration_ms: u64,
    /// 失败原因（Done 时为 None）
    #[allow(dead_code)]
    pub error: Option<String>,
}

// ── PoolMessage 构造 ──────────────────────────────────────────────────────────

impl PoolMessage {
    /// 从实时推送的 MessageEvent 构造。
    pub fn from_event(event: &MessageEvent, scope: Scope, is_bot: bool) -> Option<Self> {
        let msg_id   = event.message_id.unwrap_or(0);
        let user_id  = event.user_id;
        let timestamp = event.time.unwrap_or_else(|| crate::runtime::time::unix_timestamp());
        let nickname = extract_nickname(event.sender.as_ref());

        // 直接复用 typ::MessageSegment，不做任何转换
        let segments = event.message.clone();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, scope, user_id, nickname, timestamp,
            kind, text, segments, is_bot,
            status: MsgStatus::Pending,
            process: None,
        })
    }

    /// 从 `get_group_msg_history` 返回的原始 JSON 构造。
    /// 用于冷启动 back-seeding。
    pub fn from_api_value(value: &Value, scope: Scope) -> Option<Self> {
        let is_bot = value.get("post_type").and_then(Value::as_str) == Some("message_sent");

        let msg_id    = value.get("message_id").and_then(Value::as_i64).unwrap_or(0);
        let user_id   = value.get("user_id").and_then(Value::as_i64)?;
        let timestamp = value.get("time").and_then(Value::as_i64).unwrap_or_else(|| crate::runtime::time::unix_timestamp());

        let sender = value.get("sender");
        let card = sender.and_then(|s| s.get("card")).and_then(Value::as_str).unwrap_or("");
        let nick = sender.and_then(|s| s.get("nickname")).and_then(Value::as_str).unwrap_or("未知");
        let nickname = if card.is_empty() { nick.to_string() } else { card.to_string() };

        // 用 serde 反序列化 message 数组为 Vec<MessageSegment>，解析职责交给 typ
        let segments: Vec<MessageSegment> = value
            .get("message")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, scope, user_id, nickname, timestamp,
            kind, text, segments, is_bot,
            status: MsgStatus::Pending,
            process: None,
        })
    }
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────────

fn extract_nickname(sender: Option<&Sender>) -> String {
    let Some(s) = sender else { return "未知".to_string() };
    let card = s.card.as_deref().unwrap_or("");
    if card.is_empty() {
        s.nickname.as_deref().unwrap_or("未知").to_string()
    } else {
        card.to_string()
    }
}

/// 从消息段列表中提取并拼接所有文本内容。
///
/// 过滤出所有 text 类型的 segment 并拼接为单个字符串。
/// 若无任何文本内容则返回 None。
pub fn concat_text_segs(segments: &[MessageSegment]) -> Option<String> {
    let s: String = segments
        .iter()
        .filter_map(|seg| seg.as_text())
        .collect::<Vec<_>>()
        .join("");
    if s.is_empty() { None } else { Some(s) }
}

/// 分类消息类型。
///
/// 优先级：Reply > Forward > File > Card > Image+Text(Mixed) > Image > At > Face > Text > Other
/// - at / face 常伴随 text，不构成 Mixed
/// - 只有 image + text 同时存在才是 Mixed
pub fn classify_kind(segments: &[MessageSegment]) -> MsgKind {
    let mut has_text  = false;
    let mut has_image = false;
    let mut has_face  = false;
    let mut has_at    = false;

    for seg in segments {
        if seg.is_reply()         { return MsgKind::Reply; }
        if seg.is_forward()       { return MsgKind::Forward; }
        if seg.seg_type == "file" { return MsgKind::File; }
        if seg.seg_type == "json" { return MsgKind::Card; }
        if seg.is_text()         { has_text  = true; }
        if seg.is_image()        { has_image = true; }
        if seg.is_face()         { has_face  = true; }
        if seg.is_at()           { has_at    = true; }
    }

    if has_image && has_text { return MsgKind::Mixed; }
    if has_image             { return MsgKind::Image; }
    if has_at                { return MsgKind::At; }
    if has_face && !has_text { return MsgKind::Face; }
    if has_text              { return MsgKind::Text; }
    MsgKind::Other
}
