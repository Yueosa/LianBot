use crate::runtime::pool::MsgKind;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub user_id: i64,
    pub nickname: String,
    pub time: i64,
    pub text: String,
    pub emoji_count: u32,
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
pub(super) struct ExtractedSegments {
    pub text: String,
    pub emoji_count: u32,
    pub image_count: u32,
    pub reply_to: Option<i64>,
    pub at_targets: Vec<i64>,
    pub face_ids: Vec<String>,
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
