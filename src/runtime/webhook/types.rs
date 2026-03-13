//! Webhook 推送相关公共类型。

use std::time::Instant;

use crate::runtime::permission::Scope;
use crate::runtime::typ::MessageSegment;

/// 命令触发 → Webhook 回调的来源追踪。
///
/// 当 `!!sign` 等命令主动触发外部服务后，外部服务通过 Webhook 回调推送结果。
/// `PendingOrigin` 记录"谁触发的"，让 Service 可以将结果**回源推送**到触发者所在的群/私聊。
pub struct PendingOrigin {
    /// 触发命令的会话域
    pub scope: Scope,
    /// 创建时间，用于超时清理
    pub created_at: Instant,
}

impl PendingOrigin {
    /// 创建一个新的 pending origin。
    pub fn new(scope: Scope) -> Self {
        Self { scope, created_at: Instant::now() }
    }

    /// 是否已过期（超过 5 分钟视为过期）。
    pub fn expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > 300
    }
}

/// 构造通知消息段：@ 列表 + 换行 + 文本。
///
/// 供所有 Webhook Service 统一使用，消除重复的消息构造代码。
pub fn build_notification(text: &str, at: &[i64]) -> Vec<MessageSegment> {
    let mut segs: Vec<MessageSegment> = at.iter().map(|&qq| MessageSegment::at(qq)).collect();
    if !segs.is_empty() {
        segs.push(MessageSegment::text("\n"));
    }
    segs.push(MessageSegment::text(text));
    segs
}
