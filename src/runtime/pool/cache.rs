use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::trace;

use crate::runtime::permission::Scope;
use super::{MsgStatus, PoolConfig, PoolMessage, ProcessRecord};

// ── MemoryPool ────────────────────────────────────────────────────────────────

/// 基于 RwLock<HashMap<Scope, VecDeque<PoolMessage>>> 的内存消息缓冲。
///
/// 淘汰策略（双重触发）：
///   1. 容量淘汰：每次 push 后若 len > capacity，从队头 pop_front（最旧的先淘汰）
///   2. 时间淘汰：每次 push 后扫描队头，删除早于 (now - evict_after_secs) 的消息
///
/// 性能优化：
///   - msg_id_index: 维护 msg_id -> deque_index 的映射，实现 O(1) 去重和 mark 查找
///   - 去重策略：保留最新消息，删除旧的同 msg_id 消息
pub struct MemoryPool {
    scopes: RwLock<HashMap<Scope, VecDeque<PoolMessage>>>,
    msg_id_index: RwLock<HashMap<Scope, HashMap<i64, usize>>>,
    capacity: usize,
    evict_after_secs: i64,
}

impl MemoryPool {
    pub fn new(cfg: &PoolConfig) -> Arc<Self> {
        Arc::new(Self {
            scopes: RwLock::new(HashMap::new()),
            msg_id_index: RwLock::new(HashMap::new()),
            capacity: cfg.per_group_capacity,
            evict_after_secs: cfg.evict_after_secs,
        })
    }

    /// 写入一条消息（容量/时间淘汰自动处理）
    pub async fn push(&self, msg: PoolMessage) {
        let now = crate::runtime::time::unix_timestamp();
        let cutoff = now.saturating_sub(self.evict_after_secs);

        let scope = msg.scope;
        let msg_id = msg.msg_id;

        let mut guard = self.scopes.write().await;
        let mut index_guard = self.msg_id_index.write().await;

        let deque = guard.entry(scope).or_default();
        let index = index_guard.entry(scope).or_default();

        // msg_id 去重：如果已存在，删除旧消息，保留新消息
        if let Some(&old_idx) = index.get(&msg_id) {
            if old_idx < deque.len() {
                deque.remove(old_idx);
                // 删除后需要更新所有后续消息的索引（索引值 -1）
                for (_, idx) in index.iter_mut() {
                    if *idx > old_idx {
                        *idx -= 1;
                    }
                }
            }
        }

        // 插入新消息
        let new_idx = deque.len();
        deque.push_back(msg);
        index.insert(msg_id, new_idx);

        // 1. 时间淘汰：从队头清理过期消息
        let mut evicted_count = 0;
        while deque.front().is_some_and(|m| m.timestamp < cutoff) {
            if let Some(evicted) = deque.pop_front() {
                index.remove(&evicted.msg_id);
                evicted_count += 1;
            }
        }
        // 时间淘汰后需要更新所有索引（索引值 - evicted_count）
        if evicted_count > 0 {
            for (_, idx) in index.iter_mut() {
                *idx -= evicted_count;
            }
        }

        // 2. 容量淘汰：超出最大条数时从队头 pop
        let mut capacity_evicted = 0;
        while deque.len() > self.capacity {
            if let Some(evicted) = deque.pop_front() {
                index.remove(&evicted.msg_id);
                capacity_evicted += 1;
            }
        }
        // 容量淘汰后需要更新所有索引
        if capacity_evicted > 0 {
            for (_, idx) in index.iter_mut() {
                *idx -= capacity_evicted;
            }
        }

        trace!("[pool] {scope:?} push, depth={}", deque.len());
    }

    /// 读取指定 scope 最近 n 条消息（时序: 旧 → 新）
    ///
    /// internal-only：不保证时间连续性，仅供测试使用
    #[doc(hidden)]
    #[allow(dead_code)]
    pub async fn recent_internal(&self, scope: &Scope, n: usize) -> Vec<PoolMessage> {
        let guard = self.scopes.read().await;
        let Some(deque) = guard.get(scope) else { return vec![] };

        let len = deque.len();
        if n >= len {
            deque.iter().cloned().collect()
        } else {
            deque.iter().skip(len - n).cloned().collect()
        }
    }

    /// 读取指定 scope 在 [since, until] 时间范围内的消息（秒级时间戳）
    #[allow(dead_code)]
    pub async fn range(&self, scope: &Scope, since: i64, until: i64) -> Vec<PoolMessage> {
        let guard = self.scopes.read().await;
        let Some(deque) = guard.get(scope) else { return vec![] };

        deque.iter()
            .filter(|m| m.timestamp >= since && m.timestamp <= until)
            .cloned()
            .collect()
    }

    /// 返回指定 scope 在 pool 中最早一条消息的时间戳（秒级）
    ///
    /// 用于判断 pool 是否覆盖了某个时间窗口：若 oldest <= cutoff，则覆盖完整。
    /// 无任何消息时返回 None。
    #[allow(dead_code)]
    pub async fn oldest_timestamp(&self, scope: &Scope) -> Option<i64> {
        let guard = self.scopes.read().await;
        guard.get(scope)?.front().map(|m| m.timestamp)
    }

    /// 标记消息处理完成（dispatcher 在 cmd.execute() 之后调用）
    ///
    /// 在对应 scope 的队列中通过索引查找 msg_id 并更新状态（O(1) 查找）
    pub async fn mark(&self, msg_id: i64, scope: &Scope, status: MsgStatus, record: ProcessRecord) {
        let mut guard = self.scopes.write().await;
        let index_guard = self.msg_id_index.read().await;

        let Some(deque) = guard.get_mut(scope) else { return };
        let Some(index) = index_guard.get(scope) else { return };

        // O(1) 查找：通过索引直接定位消息
        if let Some(&idx) = index.get(&msg_id) {
            if let Some(msg) = deque.get_mut(idx) {
                msg.status = status;
                msg.process = Some(record);
            }
        }
    }

    /// 更新消息的 description 字段（用于缓存图片识别结果）
    ///
    /// 在对应 scope 的队列中通过索引查找 msg_id 并更新 description（O(1) 查找）
    pub async fn update_description(&self, msg_id: i64, scope: &Scope, description: String) {
        let mut guard = self.scopes.write().await;
        let index_guard = self.msg_id_index.read().await;

        let Some(deque) = guard.get_mut(scope) else { return };
        let Some(index) = index_guard.get(scope) else { return };

        // O(1) 查找：通过索引直接定位消息
        if let Some(&idx) = index.get(&msg_id) {
            if let Some(msg) = deque.get_mut(idx) {
                msg.description = Some(description);
            }
        }
    }

    /// 根据 msg_id 获取消息（用于检查是否已有 description）
    ///
    /// 在对应 scope 的队列中通过索引查找 msg_id（O(1) 查找）
    pub async fn get_message(&self, msg_id: i64, scope: &Scope) -> Option<PoolMessage> {
        let guard = self.scopes.read().await;
        let index_guard = self.msg_id_index.read().await;

        let deque = guard.get(scope)?;
        let index = index_guard.get(scope)?;

        // O(1) 查找：通过索引直接定位消息
        let idx = *index.get(&msg_id)?;
        deque.get(idx).cloned()
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::permission::Scope,
        runtime::pool::{MsgKind, MsgStatus, PoolConfig, PoolMessage},
        runtime::typ::message::MessageSegment,
    };

    fn make_msg(scope: Scope, uid: i64, ts: i64, text: &str) -> PoolMessage {
        PoolMessage {
            msg_id: ts,
            scope,
            user_id: uid,
            nickname: "test".into(),
            timestamp: ts,
            kind: MsgKind::Text,
            text: if text.is_empty() { None } else { Some(text.into()) },
            segments: vec![MessageSegment::text(text)],
            status: MsgStatus::Pending,
            process: None,
            is_bot: false,
            description: None,
        }
    }

    fn pool(cap: usize, evict: i64) -> Arc<MemoryPool> {
        MemoryPool::new(&PoolConfig {
            per_group_capacity: cap,
            evict_after_secs:   evict,
            ..PoolConfig::default()
        })
    }

    fn g(id: i64) -> Scope { Scope::Group(id) }

    #[tokio::test]
    async fn test_push_and_recent() {
        let p = pool(5, i64::MAX);
        for i in 1..=3 {
            p.push(make_msg(g(1), 100, i, &format!("msg{i}"))).await;
        }
        let r = p.recent_internal(&g(1), 10).await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].timestamp, 1);
        assert_eq!(r[2].timestamp, 3);
    }

    #[tokio::test]
    async fn test_capacity_eviction() {
        let p = pool(3, i64::MAX);
        for i in 1..=5 {
            p.push(make_msg(g(1), 100, i, "x")).await;
        }
        let r = p.recent_internal(&g(1), 10).await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].timestamp, 3);
    }

    #[tokio::test]
    async fn test_range() {
        let p = pool(100, i64::MAX);
        for i in 1..=10 {
            p.push(make_msg(g(1), 100, i * 1000, "x")).await;
        }
        let r = p.range(&g(1), 3000, 7000).await;
        assert_eq!(r.len(), 5);
    }

    #[tokio::test]
    async fn test_time_eviction() {
        let p = pool(100, 10);
        let now = crate::runtime::time::unix_timestamp();
        p.push(make_msg(g(1), 100, now - 20, "old")).await;
        p.push(make_msg(g(1), 100, now, "new")).await;
        let r = p.recent_internal(&g(1), 10).await;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn test_dedup_msg_id() {
        let p = pool(100, i64::MAX);
        p.push(make_msg(g(1), 100, 1, "first")).await;
        p.push(make_msg(g(1), 200, 2, "second")).await;
        p.push(make_msg(g(1), 100, 1, "dup")).await;  // 重复 msg_id=1，应删除旧的保留新的
        let r = p.recent_internal(&g(1), 10).await;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text.as_deref(), Some("second"));  // 旧的 "first" 被删除
        assert_eq!(r[1].text.as_deref(), Some("dup"));     // 新的 "dup" 保留
    }
}
