pub mod cache;

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use anyhow::Context;
use serde_json::Value;

use crate::runtime::api::ApiClient;
use crate::runtime::typ::event::{MessageEvent, Sender};
use crate::runtime::typ::message::MessageSegment;

// ── 池配置 ────────────────────────────────────────────────────────────────────

/// runtime.toml `[pool]` 段。
#[derive(Debug, Deserialize)]
pub struct PoolConfig {
    /// 每个群的内存缓冲最大消息条数，默认 3000
    #[serde(default = "PoolConfig::default_capacity")]
    pub per_group_capacity: usize,
    /// 内存淘汰阈值（秒），超过此时间的消息被清理，默认 1d
    #[serde(default = "PoolConfig::default_evict")]
    pub evict_after_secs: i64,
}

impl PoolConfig {
    fn default_capacity() -> usize { 3000 }
    fn default_evict() -> i64 { 86400 }
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
    pub group_id: i64,
    pub user_id: i64,
    /// sender.card（群名片）|| sender.nickname，已提取
    pub nickname: String,
    /// 秒级 Unix 时间戳
    pub timestamp: i64,
    pub kind: MsgKind,
    /// 所有 text segment 拼接，无文字则 None
    pub text: Option<String>,
    /// 消息段——直接复用 typ::MessageSegment，不重新定义
    pub segments: Vec<MessageSegment>,
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
    Error,
}

/// 命令处理记录（dispatcher 在 cmd.execute() 之后填充）
#[derive(Debug, Clone)]
pub struct ProcessRecord {
    /// 哪个命令处理的（纯名字，如 "acg"）
    pub command: String,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 失败原因（Done 时为 None）
    pub error: Option<String>,
}

// ── PoolMessage 构造 ──────────────────────────────────────────────────────────

impl PoolMessage {
    /// 从实时推送的 MessageEvent 构造。
    /// 非群消息返回 None。
    pub fn from_event(event: &MessageEvent, group_id: i64) -> Option<Self> {
        if !event.is_group() {
            return None;
        }

        let msg_id   = event.message_id.unwrap_or(0);
        let user_id  = event.user_id;
        let timestamp = event.time.unwrap_or_else(now_secs);
        let nickname = extract_nickname(event.sender.as_ref());

        // 直接复用 typ::MessageSegment，不做任何转换
        let segments = event.message.clone();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, group_id, user_id, nickname, timestamp,
            kind, text, segments,
            status: MsgStatus::Pending,
            process: None,
        })
    }

    /// 从 `get_group_msg_history` 返回的原始 JSON 构造。
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

        // 用 serde 反序列化 message 数组为 Vec<MessageSegment>，解析职责交给 typ
        let segments: Vec<MessageSegment> = value
            .get("message")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let text = concat_text_segs(&segments);
        let kind = classify_kind(&segments);

        Some(Self {
            msg_id, group_id, user_id, nickname, timestamp,
            kind, text, segments,
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

fn concat_text_segs(segments: &[MessageSegment]) -> Option<String> {
    let s: String = segments
        .iter()
        .filter_map(|seg| seg.as_text())
        .collect::<Vec<_>>()
        .join("");
    if s.is_empty() { None } else { Some(s) }
}

fn classify_kind(segments: &[MessageSegment]) -> MsgKind {
    let mut has_text  = false;
    let mut has_image = false;
    let mut has_face  = false;
    let mut has_at    = false;

    for seg in segments {
        if seg.is_reply()                          { return MsgKind::Reply; }
        if seg.seg_type == "file"                   { return MsgKind::File; }
        if seg.seg_type == "json"                   { return MsgKind::Card; }
        if seg.is_text()                            { has_text  = true; }
        if seg.is_image()                           { has_image = true; }
        if seg.is_face()                            { has_face  = true; }
        if seg.is_at()                              { has_at    = true; }
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

    /// 标记消息处理完成（dispatcher 在 cmd.execute() 之后调用）。
    /// 在当前群的队列中反向查找 msg_id 并更新状态。
    async fn mark(&self, msg_id: i64, group_id: i64, status: MsgStatus, record: ProcessRecord);

    /// internal-only：读取群 gid 最近 n 条消息（时序: 旧 → 新）。
    /// 不保证时间连续性，不作为命令层对外语义使用。
    #[doc(hidden)]
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
pub async fn create_pool(cfg: &PoolConfig) -> anyhow::Result<std::sync::Arc<Pool>> {
    Ok(cache::MemoryPool::new(cfg))
}

// ── 启动预热 ──────────────────────────────────────────────────────────────────

/// 拉取白名单群的历史消息填充消息池（冷启动 back-seeding）。
/// 由 boot.rs 在启动时 `tokio::spawn` 调用。
pub async fn seed_from_history(api: &ApiClient, pool: &Pool, groups: Vec<i64>) {
    if groups.is_empty() {
        tracing::info!("[pool-seed] 无已开启的群，跳过启动预热");
        return;
    }

    tracing::info!("[pool-seed] 启动预热开始：{} 个群", groups.len());
    let mut total = 0usize;

    for gid in groups {
        match seed_one_group(api, pool, gid).await {
            Ok(n) => {
                total += n;
                tracing::info!("[pool-seed] 群 {gid} 预热完成：{n} 条");
            }
            Err(e) => {
                tracing::warn!("[pool-seed] 群 {gid} 预热失败: {e:#}");
            }
        }
    }

    tracing::info!("[pool-seed] 启动预热结束：累计 {total} 条");
}

async fn seed_one_group(api: &ApiClient, pool: &Pool, group_id: i64) -> anyhow::Result<usize> {
    let raw = api
        .get_group_msg_history_paged(group_id, 3000, None)
        .await
        .with_context(|| format!("拉取群 {} 历史消息失败", group_id))?;

    let mut seeded = 0usize;
    for value in raw {
        if let Some(msg) = PoolMessage::from_api_value(&value, group_id) {
            pool.push(msg).await;
            seeded += 1;
        }
    }

    Ok(seeded)
}
