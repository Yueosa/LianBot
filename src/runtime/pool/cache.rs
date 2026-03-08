use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::debug;

use super::{MessagePool, MsgStatus, PoolConfig, PoolMessage, ProcessRecord};

// ── MemoryPool ────────────────────────────────────────────────────────────────

/// 基于 RwLock<HashMap<群号, VecDeque<PoolMessage>>> 的内存消息缓冲。
///
/// 淘汰策略（双重触发）：
///   1. 容量淘汰：每次 push 后若 len > capacity，从队头 pop_front（最旧的先淘汰）
///   2. 时间淘汰：每次 push 后扫描队头，删除早于 (now - evict_after_secs) 的消息
///
/// `recent_internal(n)` 的结果是 "capacity 内最近 n 条"，不保证跨越超长时间窗口。
pub struct MemoryPool {
    groups: RwLock<HashMap<i64, VecDeque<PoolMessage>>>,
    capacity: usize,
    evict_after_secs: i64,
}

impl MemoryPool {
    pub fn new(cfg: &PoolConfig) -> Arc<Self> {
        Arc::new(Self {
            groups: RwLock::new(HashMap::new()),
            capacity: cfg.per_group_capacity,
            evict_after_secs: cfg.evict_after_secs,
        })
    }
}

#[async_trait]
impl MessagePool for MemoryPool {
    async fn push(&self, msg: PoolMessage) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cutoff = now.saturating_sub(self.evict_after_secs);

        let gid = msg.group_id;
        let mut guard = self.groups.write().await;
        let deque = guard.entry(gid).or_default();

        // msg_id 去重：已存在则跳过（back_seed 场景）
        if deque.iter().rev().any(|m| m.msg_id == msg.msg_id) {
            return;
        }

        deque.push_back(msg);

        // 1. 时间淘汰：从队头清理过期消息
        while deque.front().is_some_and(|m| m.timestamp < cutoff) {
            deque.pop_front();
        }

        // 2. 容量淘汰：超出最大条数时从队头 pop
        while deque.len() > self.capacity {
            deque.pop_front();
        }

        debug!("[pool] 群 {gid} push, depth={}", deque.len());
    }

    async fn recent_internal(&self, gid: i64, n: usize) -> Vec<PoolMessage> {
        let guard = self.groups.read().await;
        let Some(deque) = guard.get(&gid) else { return vec![] };

        // 取最后 n 条，按时序 旧→新 返回
        deque.iter()
            .rev()
            .take(n)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    async fn range(&self, gid: i64, since: i64, until: i64) -> Vec<PoolMessage> {
        let guard = self.groups.read().await;
        let Some(deque) = guard.get(&gid) else { return vec![] };

        deque.iter()
            .filter(|m| m.timestamp >= since && m.timestamp <= until)
            .cloned()
            .collect()
    }

    async fn oldest_timestamp(&self, gid: i64) -> Option<i64> {
        let guard = self.groups.read().await;
        guard.get(&gid)?.front().map(|m| m.timestamp)
    }

    async fn mark(&self, msg_id: i64, group_id: i64, status: MsgStatus, record: ProcessRecord) {
        let mut guard = self.groups.write().await;
        let Some(deque) = guard.get_mut(&group_id) else { return };

        // 从队尾反向查找（最近的消息在队尾，绝大多数场景 O(1)~O(几)）
        for msg in deque.iter_mut().rev() {
            if msg.msg_id == msg_id {
                msg.status = status;
                msg.process = Some(record);
                return;
            }
        }
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::pool::{MsgKind, MsgStatus, PoolConfig, PoolMessage},
        runtime::typ::message::MessageSegment,
    };

    fn make_msg(gid: i64, uid: i64, ts: i64, text: &str) -> PoolMessage {
        PoolMessage {
            msg_id: ts,
            group_id: gid,
            user_id: uid,
            nickname: "test".into(),
            timestamp: ts,
            kind: MsgKind::Text,
            text: if text.is_empty() { None } else { Some(text.into()) },
            segments: vec![MessageSegment::text(text)],
            status: MsgStatus::Pending,
            process: None,
        }
    }

    fn pool(cap: usize, evict: i64) -> Arc<MemoryPool> {
        MemoryPool::new(&PoolConfig {
            per_group_capacity: cap,
            evict_after_secs:   evict,
            ..PoolConfig::default()
        })
    }

    #[tokio::test]
    async fn test_push_and_recent() {
        let p = pool(5, i64::MAX); // 不测试时间淘汰（saturating_sub → cutoff ≤ 0）
        for i in 1..=3 {
            p.push(make_msg(1, 100, i, &format!("msg{i}"))).await;
        }
        let r = p.recent_internal(1, 10).await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].timestamp, 1); // 旧 → 新
        assert_eq!(r[2].timestamp, 3);
    }

    #[tokio::test]
    async fn test_capacity_eviction() {
        let p = pool(3, i64::MAX); // 不测试时间淘汰
        for i in 1..=5 {
            p.push(make_msg(1, 100, i, "x")).await;
        }
        let r = p.recent_internal(1, 10).await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].timestamp, 3); // 最旧的 1,2 被淘汰
    }

    #[tokio::test]
    async fn test_range() {
        let p = pool(100, i64::MAX); // 不测试时间淘汰
        for i in 1..=10 {
            p.push(make_msg(1, 100, i * 1000, "x")).await;
        }
        let r = p.range(1, 3000, 7000).await;
        assert_eq!(r.len(), 5); // ts=3000,4000,5000,6000,7000
    }

    #[tokio::test]
    async fn test_time_eviction() {
        let p = pool(100, 10); // evict > 10s
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        p.push(make_msg(1, 100, now - 20, "old")).await; // 过期
        p.push(make_msg(1, 100, now, "new")).await;      // 新消息触发淘汰
        let r = p.recent_internal(1, 10).await;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn test_dedup_msg_id() {
        let p = pool(100, i64::MAX);
        // 推入两条不同消息
        p.push(make_msg(1, 100, 1, "first")).await;
        p.push(make_msg(1, 200, 2, "second")).await;
        // 重复 msg_id=1，应被拒绝
        p.push(make_msg(1, 100, 1, "dup")).await;
        let r = p.recent_internal(1, 10).await;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text.as_deref(), Some("first"));
        assert_eq!(r[1].text.as_deref(), Some("second"));
    }
}
