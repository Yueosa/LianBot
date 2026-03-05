pub mod cache;

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::typ::event::{MessageEvent, Sender};

// ── PoolMessage 及相关类型 ────────────────────────────────────────────────────

/// 消息池中存储的统一消息结构。
/// 不存储 NapCat raw 字段，精简内存占用。
#[derive(Debug, Clone)]
pub struct PoolMessage {
    /// OneBot message_id（接近 i32::MAX，使用 i64）
    pub msg_id: i64,
    pub group_id: i64,
    pub user_id: i64,
    /// sender.card（群名片）|| sender.nickname，已提取
    pub nickname: String,
    /// 秒级 Unix 时间戳
    pub timestamp: i64,
    pub kind: MsgKind,
    /// 所有 text segment 拼接（含混合消息文字部分），无文字则 None
    pub text: Option<String>,
    /// 解析好的消息段，不含 raw
    pub segments: Vec<Segment>,
    /// AI 流水线预留字段（当前未使用）
    #[allow(dead_code)]
    pub status: MsgStatus,
    #[allow(dead_code)]
    pub processing: Option<ProcessInfo>,
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
    Mixed,  // 多类型混合（如 text + image）
    Other,  // 未分类
}

/// 单个消息段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub kind: String,
    pub data: Value,
}

/// 消息处理状态（为 AI 流水线预留，当前仅使用 Pending）
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgStatus {
    Pending,
    Processing,
    Done,
    Error,
}

/// 处理信息（供 AI 模块记录，当前尚未使用）
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub module: String,
    pub detail: String,
    pub context: Option<Value>,
}

// ── PoolMessage 构造 ──────────────────────────────────────────────────────────

impl PoolMessage {
    /// 从实时推送的 MessageEvent 构造。
    /// 非群消息返回 None（private / notice 等不入池）。
    pub fn from_event(event: &MessageEvent, group_id: i64) -> Option<Self> {
        if !event.is_group() {
            return None;
        }

        let msg_id   = event.message_id.unwrap_or(0);
        let user_id  = event.user_id;
        let timestamp = event.time.unwrap_or_else(now_secs);
        let nickname = extract_nickname(event.sender.as_ref());

        let segments: Vec<Segment> = event.message.iter().map(|seg| Segment {
            kind: seg.seg_type.clone(),
            data: seg.data.clone(),
        }).collect();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, group_id, user_id, nickname, timestamp,
            kind, text, segments,
            status: MsgStatus::Pending,
            processing: None,
        })
    }

    /// 从 `get_group_msg_history` 返回的原始 JSON 构造（与推送格式一致）。
    /// 用于冷启动 back-seeding。
    pub fn from_api_value(value: &Value, group_id: i64) -> Option<Self> {
        // 跳过 Bot 自身发送的消息
        if value.get("post_type").and_then(Value::as_str) == Some("message_sent") {
            return None;
        }

        let msg_id    = value.get("message_id").and_then(Value::as_i64).unwrap_or(0);
        let user_id   = value.get("user_id").and_then(Value::as_i64)?;
        let timestamp = value.get("time").and_then(Value::as_i64).unwrap_or_else(now_secs);

        let sender = value.get("sender");
        let card = sender.and_then(|s| s.get("card")).and_then(Value::as_str).unwrap_or("");
        let nick = sender.and_then(|s| s.get("nickname")).and_then(Value::as_str).unwrap_or("未知");
        let nickname = if card.is_empty() { nick.to_string() } else { card.to_string() };

        let segments: Vec<Segment> = value
            .get("message")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter().filter_map(|seg| {
                    let kind = seg.get("type").and_then(Value::as_str)?.to_string();
                    let data = seg.get("data").cloned().unwrap_or(Value::Null);
                    Some(Segment { kind, data })
                })
                .collect()
            })
            .unwrap_or_default();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, group_id, user_id, nickname, timestamp,
            kind, text, segments,
            status: MsgStatus::Pending,
            processing: None,
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

fn concat_text_segs(segments: &[Segment]) -> Option<String> {
    let s: String = segments
        .iter()
        .filter(|seg| seg.kind == "text")
        .filter_map(|seg| seg.data.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    if s.is_empty() { None } else { Some(s) }
}

fn classify_kind(segments: &[Segment]) -> MsgKind {
    let mut has_text  = false;
    let mut has_image = false;
    let mut has_face  = false;
    let mut has_at    = false;

    for seg in segments {
        match seg.kind.as_str() {
            "reply"                             => return MsgKind::Reply,
            "file"                              => return MsgKind::File,
            "json"                              => return MsgKind::Card,
            "text"                              => has_text  = true,
            "image"                             => has_image = true,
            "face"|"mface"|"bface"|"sface"      => has_face  = true,
            "at"                                => has_at    = true,
            _ => {}
        }
    }

    // 多类型混合
    let type_count = [has_text, has_image, has_face, has_at].iter().filter(|&&b| b).count();
    if type_count > 1 {
        return MsgKind::Mixed;
    }

    match (has_text, has_image, has_face, has_at) {
        (true,  false, false, false) => MsgKind::Text,
        (false, true,  false, _   ) => MsgKind::Image,
        (false, false, true,  _   ) => MsgKind::Face,
        (_,     _,     _,     true ) => MsgKind::At,
        _ => MsgKind::Other,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── MessagePool Trait ─────────────────────────────────────────────────────────

/// 消息池统一接口。当前实现为 InMemory（后续可扩展为 Actor/其他存储后端）。
#[async_trait]
pub trait MessagePool: Send + Sync {
    /// 写入一条消息（容量/时间淘汰由实现层自动处理）
    async fn push(&self, msg: PoolMessage);

    /// internal-only：读取群 gid 最近 n 条消息（时序: 旧 → 新）。
    /// 不保证时间连续性，不作为命令层对外语义使用。
    #[doc(hidden)]
    #[allow(dead_code)]
    async fn recent_internal(&self, gid: i64, n: usize) -> Vec<PoolMessage>;

    /// 读取群 gid 在 [since, until] 时间范围内的消息（秒级时间戳）
    async fn range(&self, gid: i64, since: i64, until: i64) -> Vec<PoolMessage>;

    /// 返回群 gid 在 pool 中最早一条消息的时间戳（秒级）。
    /// 用于判断 pool 是否覆盖了某个时间窗口：若 oldest <= cutoff，则覆盖完整。
    /// 无任何消息时返回 None。
    async fn oldest_timestamp(&self, gid: i64) -> Option<i64>;
}

// ── 类型别名 & 工厂函数 ────────────────────────────────────────────────────────

/// 消息池的具体实现类型（当前固定为 MemoryPool）。
pub type Pool = cache::MemoryPool;

/// 统一的消息池创建入口，在 `main.rs` 中调用。
/// 当前：MemoryPool
pub async fn create_pool(cfg: &crate::kernel::config::PoolConfig) -> anyhow::Result<std::sync::Arc<Pool>> {
    Ok(cache::MemoryPool::new(cfg))
}
