use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Context as _};
use rusqlite::Connection;
use tokio::task;
use tracing::debug;

use super::MigrationFn;

/// 通用 SQLite 句柄。
///
/// - 内部持有单个 `Connection`，用 `Arc<Mutex<_>>` 序列化所有 Rust 侧访问。
/// - 所有 IO 通过 [`SqliteDb::call`] 在 blocking 线程池执行。
/// - `Clone` 只复制 `Arc`，底层连接共享。
#[derive(Clone)]
pub struct SqliteDb {
    conn: Arc<Mutex<Connection>>,
}

impl fmt::Debug for SqliteDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteDb").finish_non_exhaustive()
    }
}

impl SqliteDb {
    /// 打开数据库文件，执行 PRAGMA 初始化，并按需运行版本迁移。
    ///
    /// `migrations` 的顺序即版本号：index 0 对应 v1，index 1 对应 v2，以此类推。
    /// 已执行过的版本自动跳过；失败时回滚并返回错误，不会留下半完成的 schema。
    pub fn open(path: &Path, migrations: &[MigrationFn]) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("open db: {}", path.display()))?;
        setup_pragmas(&conn)?;
        migrate(&conn, migrations)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 在 blocking 线程上执行同步 DB 操作，外层加 10 s 超时。
    ///
    /// # 错误
    /// - 超时（10 s）→ `"db operation timed out"`
    /// - `Mutex` 中毒（不应发生）→ `"db mutex poisoned"`
    /// - 任务 panic → `"db task panicked"`
    /// - 闭包内 `anyhow::Error` 直接透传
    pub async fn call<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&Connection) -> anyhow::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let handle = task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|_| anyhow!("db mutex poisoned"))?;
            f(&guard)
        });
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .context("db operation timed out")? // Elapsed → anyhow
            .context("db task panicked")? // JoinError → anyhow
    }
}

// ── 内部实现 ──────────────────────────────────────────────────────────────────

fn setup_pragmas(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(concat!(
        "PRAGMA journal_mode = WAL;",
        "PRAGMA synchronous   = NORMAL;",
        "PRAGMA busy_timeout  = 5000;",
        "PRAGMA foreign_keys  = ON;",
    ))
    .context("setup db pragmas")?;
    Ok(())
}

fn migrate(conn: &Connection, migrations: &[MigrationFn]) -> anyhow::Result<()> {
    // 建立版本追踪表（首次运行时创建，已存在则跳过）
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .context("create meta table")?;

    let mut current: i64 = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    for (i, mig) in migrations.iter().enumerate() {
        let target = (i + 1) as i64;
        if current >= target {
            continue;
        }

        debug!("running db migration v{target}");

        // 每个迁移步骤在独立事务内完成
        conn.execute_batch("BEGIN;")
            .with_context(|| format!("begin migration v{target}"))?;

        let result = mig(conn).and_then(|_| {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
                rusqlite::params![target.to_string()],
            )
            .map(|_| ())
        });

        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT;")
                    .with_context(|| format!("commit migration v{target}"))?;
                current = target;
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e).with_context(|| format!("db migration v{target} failed"));
            }
        }
    }

    Ok(())
}
