// 仅在 core-pool-sqlite feature 启用时编译
// 通过 pool/mod.rs 中 #[cfg(feature = "core-pool-sqlite")] pub mod store; 控制

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{Connection, params};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::{
    MessagePool, MsgKind, MsgStatus, PoolMessage, Segment,
    cache::MemoryPool,
};
use crate::core::config::PoolConfig;

// ── 常量 ──────────────────────────────────────────────────────────────────────

/// 后台写入 channel 容量（写满时 try_send 失败，丢弃并发出警告）
const CHANNEL_CAP: usize = 4096;

/// 每次事务最多批量写入的消息条数
const BATCH_SIZE: usize = 100;

/// 定期清理间隔（秒）
const CLEANUP_INTERVAL_SECS: u64 = 3600;

// ── HybridPool ────────────────────────────────────────────────────────────────

/// 内存 + SQLite 混合消息池。
///
/// - **写入路径**：先写内存（同步），再经 mpsc channel 异步写入 SQLite（低优先级，写满时丢弃）
/// - **读取路径**：优先读内存；内存未命中则查 SQLite；两者均空时返回空（上层回退 API）
/// - **SQLite 保留策略**：优先 30 天时间淘汰 + 每群 50,000 条行数上限
pub struct HybridPool {
    memory: Arc<MemoryPool>,
    tx: mpsc::Sender<PoolMessage>,
    db_path: Arc<str>,
}

impl HybridPool {
    pub async fn new(cfg: &PoolConfig) -> Result<Arc<Self>> {
        let memory = MemoryPool::new(cfg);
        let (tx, rx) = mpsc::channel::<PoolMessage>(CHANNEL_CAP);
        let db_path: Arc<str> = Arc::from(cfg.sqlite_path.as_str());

        // 初始化 schema（阻塞操作，在 spawn_blocking 中执行）
        {
            let path = cfg.sqlite_path.clone();
            tokio::task::spawn_blocking(move || init_schema(&path))
                .await
                .context("spawn_blocking 失败")?
                .context("SQLite schema 初始化失败")?;
        }

        // 启动时执行一次清理
        {
            let path   = cfg.sqlite_path.clone();
            let days   = cfg.sqlite_retain_days;
            let maxr   = cfg.sqlite_max_rows_per_group;
            tokio::task::spawn_blocking(move || cleanup(&path, days, maxr))
                .await
                .context("spawn_blocking 失败")?
                .context("SQLite 启动清理失败")?;
        }

        // 启动后台 writer task
        {
            let path   = cfg.sqlite_path.clone();
            let days   = cfg.sqlite_retain_days;
            let maxr   = cfg.sqlite_max_rows_per_group;
            tokio::spawn(writer_task(rx, path, days, maxr));
        }

        info!("[pool] HybridPool 初始化完成，SQLite: {}", cfg.sqlite_path);
        Ok(Arc::new(Self { memory, tx, db_path }))
    }
}

#[async_trait]
impl MessagePool for HybridPool {
    async fn push(&self, msg: PoolMessage) {
        // 1. 同步写内存（快）
        self.memory.push(msg.clone()).await;
        // 2. 异步投递到 SQLite 写入队列（低优先级，写满丢弃）
        if self.tx.try_send(msg).is_err() {
            warn!("[pool] SQLite 写入队列已满（cap={}），丢弃一条消息", CHANNEL_CAP);
        }
    }

    async fn recent(&self, gid: i64, n: usize) -> Vec<PoolMessage> {
        // 1. 内存优先
        let cached = self.memory.recent(gid, n).await;
        if !cached.is_empty() {
            return cached;
        }
        // 2. 回落 SQLite
        let path = Arc::clone(&self.db_path);
        match tokio::task::spawn_blocking(move || sqlite_recent(&path, gid, n)).await {
            Ok(Ok(rows)) => {
                debug!("[pool] SQLite recent: gid={gid} n={n} → {} 条", rows.len());
                rows
            }
            Ok(Err(e)) => { warn!("[pool] SQLite recent 查询失败: {e:#}"); vec![] }
            Err(e)     => { warn!("[pool] spawn_blocking 失败: {e}"); vec![] }
        }
    }

    async fn range(&self, gid: i64, since: i64, until: i64) -> Vec<PoolMessage> {
        // 1. 内存优先
        let cached = self.memory.range(gid, since, until).await;
        if !cached.is_empty() {
            return cached;
        }
        // 2. 回落 SQLite
        let path = Arc::clone(&self.db_path);
        match tokio::task::spawn_blocking(move || sqlite_range(&path, gid, since, until)).await {
            Ok(Ok(rows)) => {
                debug!("[pool] SQLite range: gid={gid} since={since} until={until} → {} 条", rows.len());
                rows
            }
            Ok(Err(e)) => { warn!("[pool] SQLite range 查询失败: {e:#}"); vec![] }
            Err(e)     => { warn!("[pool] spawn_blocking 失败: {e}"); vec![] }
        }
    }
}

// ── 后台 writer task ──────────────────────────────────────────────────────────

/// 持续从 channel 接收消息，批量写入 SQLite，并定期清理过期记录。
async fn writer_task(
    mut rx: mpsc::Receiver<PoolMessage>,
    db_path: String,
    retain_days: u32,
    max_rows_per_group: usize,
) {
    let mut last_cleanup = std::time::Instant::now();

    loop {
        // 等第一条
        let first = match rx.recv().await {
            Some(m) => m,
            None    => break, // channel 关闭，退出
        };

        // 攒一批（最多 BATCH_SIZE 条，不等待）
        let mut batch = Vec::with_capacity(BATCH_SIZE);
        batch.push(first);
        while batch.len() < BATCH_SIZE {
            match rx.try_recv() {
                Ok(m)  => batch.push(m),
                Err(_) => break,
            }
        }

        let path = db_path.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || insert_batch(&path, &batch)).await {
            warn!("[pool] SQLite 批量写入失败: {e}");
        }

        // 定期清理（比每小时多一点宽松：自上次超过阈值才触发）
        if last_cleanup.elapsed().as_secs() > CLEANUP_INTERVAL_SECS {
            let path = db_path.clone();
            match tokio::task::spawn_blocking(move || cleanup(&path, retain_days, max_rows_per_group)).await {
                Ok(Ok(_))  => info!("[pool] SQLite 定期清理完成"),
                Ok(Err(e)) => warn!("[pool] SQLite 定期清理失败: {e:#}"),
                Err(e)     => warn!("[pool] spawn_blocking 清理失败: {e}"),
            }
            last_cleanup = std::time::Instant::now();
        }
    }

    info!("[pool] SQLite writer task 已退出");
}

// ── Schema 初始化 ─────────────────────────────────────────────────────────────

fn init_schema(path: &str) -> Result<()> {
    let conn = open(path)?;
    // WAL 模式：写入者不阻塞读取者，适合高频写低延迟读场景
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?; // 平衡安全与速度
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            msg_id    INTEGER,
            group_id  INTEGER NOT NULL,
            user_id   INTEGER NOT NULL,
            nickname  TEXT    NOT NULL,
            timestamp INTEGER NOT NULL,
            kind      TEXT    NOT NULL,
            text      TEXT,
            segments  TEXT    NOT NULL,
            PRIMARY KEY (group_id, msg_id)
        );
        CREATE INDEX IF NOT EXISTS idx_gid_ts ON messages(group_id, timestamp);",
    )?;
    Ok(())
}

// ── 写入 ──────────────────────────────────────────────────────────────────────

fn insert_batch(path: &str, batch: &[PoolMessage]) -> Result<()> {
    let mut conn = open(path)?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO messages
             (msg_id, group_id, user_id, nickname, timestamp, kind, text, segments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for msg in batch {
            let segments_json = serde_json::to_string(&msg.segments)
                .unwrap_or_else(|_| "[]".to_string());
            stmt.execute(params![
                msg.msg_id,
                msg.group_id,
                msg.user_id,
                msg.nickname,
                msg.timestamp,
                kind_str(&msg.kind),
                msg.text,
                segments_json,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

// ── 读取 ──────────────────────────────────────────────────────────────────────

fn sqlite_recent(path: &str, gid: i64, n: usize) -> Result<Vec<PoolMessage>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT msg_id, group_id, user_id, nickname, timestamp, kind, text, segments
         FROM messages
         WHERE group_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2",
    )?;
    let mut rows: Vec<PoolMessage> = stmt
        .query_map(params![gid, n as i64], row_to_msg)?
        .filter_map(|r| r.ok().flatten())
        .collect();
    // query 按 DESC，翻转为 旧→新
    rows.reverse();
    Ok(rows)
}

fn sqlite_range(path: &str, gid: i64, since: i64, until: i64) -> Result<Vec<PoolMessage>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT msg_id, group_id, user_id, nickname, timestamp, kind, text, segments
         FROM messages
         WHERE group_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
         ORDER BY timestamp ASC",
    )?;
    let rows: Vec<PoolMessage> = stmt
        .query_map(params![gid, since, until], row_to_msg)?
        .filter_map(|r| r.ok().flatten())
        .collect();
    Ok(rows)
}

fn row_to_msg(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PoolMessage>> {
    let msg_id:   i64    = row.get(0)?;
    let group_id: i64    = row.get(1)?;
    let user_id:  i64    = row.get(2)?;
    let nickname: String = row.get(3)?;
    let timestamp: i64   = row.get(4)?;
    let kind_s:   String = row.get(5)?;
    let text:     Option<String> = row.get(6)?;
    let segs_json: String = row.get(7)?;

    let segments: Vec<Segment> = serde_json::from_str(&segs_json).unwrap_or_default();
    let kind = str_kind(&kind_s);

    Ok(Some(PoolMessage {
        msg_id, group_id, user_id, nickname, timestamp,
        kind, text, segments,
        status: MsgStatus::Pending,
        processing: None,
    }))
}

// ── 清理 ──────────────────────────────────────────────────────────────────────

/// 两阶段清理：
///   1. 删除超过 retain_days 天的旧记录（跨所有群）
///   2. 对每个群，若行数超过 max_rows，删除最旧的记录直到不超过上限
fn cleanup(path: &str, retain_days: u32, max_rows_per_group: usize) -> Result<()> {
    let conn = open(path)?;

    // 1. 时间淘汰
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now_ts - (retain_days as i64) * 86400;
    let deleted: usize = conn.execute(
        "DELETE FROM messages WHERE timestamp < ?1",
        params![cutoff],
    )?;
    if deleted > 0 {
        info!("[pool] SQLite 清理过期记录: {} 条（> {}d）", deleted, retain_days);
    }

    // 2. 按群行数上限淘汰
    // 找出超出上限的群，删除最旧超出部分
    let groups: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT group_id FROM messages GROUP BY group_id HAVING count(*) > ?1",
        )?;
        stmt.query_map(params![max_rows_per_group as i64], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    };
    for gid in groups {
        // 找到第 max_rows+1 条（最旧那批）的时间戳，删除比它更旧的
        let cutoff_ts: Option<i64> = conn.query_row(
            "SELECT timestamp FROM messages WHERE group_id = ?1
             ORDER BY timestamp DESC LIMIT 1 OFFSET ?2",
            params![gid, max_rows_per_group as i64],
            |r| r.get(0),
        ).ok();
        if let Some(ts) = cutoff_ts {
            let n: usize = conn.execute(
                "DELETE FROM messages WHERE group_id = ?1 AND timestamp <= ?2",
                params![gid, ts],
            )?;
            if n > 0 {
                info!("[pool] SQLite 行数上限清理: group={gid} 删除 {n} 条");
            }
        }
    }

    Ok(())
}

// ── 辅助 ──────────────────────────────────────────────────────────────────────

fn open(path: &str) -> Result<Connection> {
    Connection::open(path).context("无法打开 SQLite 数据库")
}

fn kind_str(kind: &MsgKind) -> &'static str {
    match kind {
        MsgKind::Text  => "text",
        MsgKind::Image => "image",
        MsgKind::Face  => "face",
        MsgKind::Reply => "reply",
        MsgKind::At    => "at",
        MsgKind::Card  => "card",
        MsgKind::File  => "file",
        MsgKind::Mixed => "mixed",
        MsgKind::Other => "other",
    }
}

fn str_kind(s: &str) -> MsgKind {
    match s {
        "text"  => MsgKind::Text,
        "image" => MsgKind::Image,
        "face"  => MsgKind::Face,
        "reply" => MsgKind::Reply,
        "at"    => MsgKind::At,
        "card"  => MsgKind::Card,
        "file"  => MsgKind::File,
        "mixed" => MsgKind::Mixed,
        _       => MsgKind::Other,
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::{
        config::PoolConfig,
        pool::{MsgKind, MsgStatus, PoolMessage, Segment},
    };

    fn test_cfg(path: &str) -> PoolConfig {
        PoolConfig {
            per_group_capacity: 100,
            evict_after_secs: i64::MAX,
            sqlite_path: path.to_string(),
            sqlite_retain_days: 30,
            sqlite_max_rows_per_group: 50_000,
        }
    }

    fn make_msg(gid: i64, uid: i64, ts: i64, text: &str) -> PoolMessage {
        PoolMessage {
            msg_id: ts,
            group_id: gid,
            user_id: uid,
            nickname: "test".into(),
            timestamp: ts,
            kind: MsgKind::Text,
            text: if text.is_empty() { None } else { Some(text.into()) },
            segments: vec![Segment {
                kind: "text".into(),
                data: serde_json::json!({"text": text}),
            }],
            status: MsgStatus::Pending,
            processing: None,
        }
    }

    #[tokio::test]
    async fn test_hybrid_push_recent() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cfg = test_cfg(tmp.path().to_str().unwrap());
        let pool = HybridPool::new(&cfg).await.unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for i in 1..=5i64 {
            pool.push(make_msg(1, 100, now - (5 - i), &format!("msg{i}"))).await;
        }
        // 等后台写入
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let r = pool.recent(1, 10).await;
        assert_eq!(r.len(), 5, "内存应命中 5 条");

        // 用相同路径创建新实例（内存空），验证 SQLite 回退
        let pool2 = HybridPool::new(&cfg).await.unwrap();
        let r2 = pool2.recent(1, 10).await;
        assert_eq!(r2.len(), 5, "SQLite 应返回 5 条");
        assert_eq!(r2[0].timestamp, r[0].timestamp, "时序一致");
    }

    #[tokio::test]
    async fn test_hybrid_range() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cfg = test_cfg(tmp.path().to_str().unwrap());
        let pool = HybridPool::new(&cfg).await.unwrap();

        let base = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0) - 100;
        for i in 1..=10i64 {
            pool.push(make_msg(1, 100, base + i * 10, "x")).await;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let pool2 = HybridPool::new(&cfg).await.unwrap();
        let since = base + 30; // 包含 i=3..=7（30,40,50,60,70）
        let until = base + 70;
        let r = pool2.range(1, since, until).await;
        assert_eq!(r.len(), 5, "应返回 5 条（ts=base+30~70）");
    }
}
