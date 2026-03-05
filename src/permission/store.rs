use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use tracing::info;

use crate::db::SqliteDb;

use super::model::{BotUser, Role, Scope, Status};

// ── 内部缓存结构 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct GroupPolicy {
    enabled:  bool,
    /// 预留：P3 llm 自由发言开关
    #[allow(dead_code)]
    llm_free: bool,
}

/// 内存中一条被 Block 用户记录的 key。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BlockKey {
    /// 0 = global，1 = group
    scope_type: u8,
    scope_id:   i64,
    user_id:    i64,
}

#[derive(Debug, Default)]
struct PermState {
    groups:  HashMap<i64, GroupPolicy>,
    blocked: HashSet<BlockKey>,
}

// ── PermissionStore ───────────────────────────────────────────────────────────

/// 权限存储：内存缓存（热路径）+ SQLite 持久化（写穿）。
///
/// 所有读操作走内存缓存，无 DB I/O。
/// 所有写操作先更新内存，再异步写穿到 SQLite。
pub struct PermissionStore {
    db:    SqliteDb,
    state: RwLock<PermState>,
}

impl PermissionStore {
    /// 打开权限数据库，加载缓存，并将 `initial_groups` 中尚不存在的群写入。
    pub async fn open(path: &Path, initial_groups: &[i64]) -> anyhow::Result<Arc<Self>> {
        // 1. 打开 SqliteDb（在 blocking 线程里执行 open+migrate）
        let path_buf = path.to_path_buf();
        let db = tokio::task::spawn_blocking(move || {
            SqliteDb::open(&path_buf, &[migration_v1])
        })
        .await
        .context("db open task panicked")?
        .context("open permissions.db")?;

        // 2. 导入初始群（INSERT OR IGNORE）
        if !initial_groups.is_empty() {
            let groups = initial_groups.to_vec();
            let now = unix_now();
            db.call(move |conn| {
                for gid in &groups {
                    conn.execute(
                        "INSERT OR IGNORE INTO group_policy (group_id, enabled, llm_free, updated_at) \
                         VALUES (?1, 1, 0, ?2)",
                        rusqlite::params![gid, now],
                    )?;
                }
                Ok(())
            })
            .await
            .context("import initial_groups")?;
        }

        // 3. 全量 load groups
        let groups: HashMap<i64, GroupPolicy> = db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT group_id, enabled, llm_free FROM group_policy",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            GroupPolicy {
                                enabled:  row.get::<_, i64>(1)? != 0,
                                llm_free: row.get::<_, i64>(2)? != 0,
                            },
                        ))
                    })?
                    .collect::<rusqlite::Result<HashMap<_, _>>>()?;
                Ok(rows)
            })
            .await
            .context("load group_policy")?;

        // 4. 全量 load blocked users
        let blocked: HashSet<BlockKey> = db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT scope_type, scope_id, user_id FROM user_status WHERE status = 'blocked'",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        let st: String = row.get(0)?;
                        Ok(BlockKey {
                            scope_type: if st == "global" { 0 } else { 1 },
                            scope_id:   row.get(1)?,
                            user_id:    row.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<HashSet<_>>>()?;
                Ok(rows)
            })
            .await
            .context("load user_status")?;

        info!(
            "[perm] 已加载 {} 个群策略，{} 条黑名单",
            groups.len(),
            blocked.len()
        );

        Ok(Arc::new(Self {
            db,
            state: RwLock::new(PermState { groups, blocked }),
        }))
    }

    // ── 读操作（纯内存，同步）────────────────────────────────────────────────

    /// 该群是否对 Bot 开放。
    pub fn is_group_enabled(&self, group_id: i64) -> bool {
        self.state
            .read()
            .expect("perm RwLock poisoned")
            .groups
            .get(&group_id)
            .map(|p| p.enabled)
            .unwrap_or(false)
    }

    /// 返回所有已开启的群号（供 pool 预热等使用）。
    pub fn enabled_groups(&self) -> Vec<i64> {
        self.state
            .read()
            .expect("perm RwLock poisoned")
            .groups
            .iter()
            .filter(|(_, p)| p.enabled)
            .map(|(gid, _)| *gid)
            .collect()
    }

    /// 根据 QQ 信息构造 `BotUser`（全局 + scope 两层合并，Owner 永远 Normal）。
    pub fn resolve_user(&self, owner_id: i64, user_id: i64, scope: Scope) -> BotUser {
        if user_id == owner_id {
            return BotUser {
                user_id,
                scope,
                role:   Role::Owner,
                status: Status::Normal,
            };
        }

        let state = self.state.read().expect("perm RwLock poisoned");

        // 全局黑名单
        let global_blocked = state.blocked.contains(&BlockKey {
            scope_type: 0,
            scope_id:   0,
            user_id,
        });

        // scope 级黑名单
        let scope_blocked = match &scope {
            Scope::Group(gid) => state.blocked.contains(&BlockKey {
                scope_type: 1,
                scope_id:   *gid,
                user_id,
            }),
            Scope::Private(_) => false,
        };

        let status = if global_blocked || scope_blocked {
            Status::Blocked
        } else {
            Status::Normal
        };

        BotUser { user_id, scope, role: Role::Member, status }
    }

    // ── 写操作（先写内存，再写穿 DB）──────────────────────────────────────────

    /// 将用户在指定 scope（或全局）加入黑名单。
    pub async fn block_user(
        &self,
        scope: &Scope,
        user_id: i64,
    ) -> anyhow::Result<()> {
        let (scope_type_str, scope_type_u8, scope_id) = scope_parts(scope);
        let key = BlockKey { scope_type: scope_type_u8, scope_id, user_id };

        // 先写内存
        self.state
            .write()
            .expect("perm RwLock poisoned")
            .blocked
            .insert(key);

        // 再写穿 DB
        let now = unix_now();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO user_status \
                     (scope_type, scope_id, user_id, status, updated_at) \
                     VALUES (?1, ?2, ?3, 'blocked', ?4)",
                    rusqlite::params![scope_type_str, scope_id, user_id, now],
                )?;
                Ok(())
            })
            .await
            .context("block_user: db write")
    }

    /// 解除用户在指定 scope（或全局）的黑名单。
    pub async fn unblock_user(
        &self,
        scope: &Scope,
        user_id: i64,
    ) -> anyhow::Result<()> {
        let (scope_type_str, scope_type_u8, scope_id) = scope_parts(scope);
        let key = BlockKey { scope_type: scope_type_u8, scope_id, user_id };

        self.state
            .write()
            .expect("perm RwLock poisoned")
            .blocked
            .remove(&key);

        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM user_status \
                     WHERE scope_type = ?1 AND scope_id = ?2 AND user_id = ?3",
                    rusqlite::params![scope_type_str, scope_id, user_id],
                )?;
                Ok(())
            })
            .await
            .context("unblock_user: db write")
    }
}

// ── 迁移函数 ──────────────────────────────────────────────────────────────────

/// v1 schema：group_policy + user_status
fn migration_v1(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE group_policy (
            group_id   INTEGER PRIMARY KEY,
            enabled    INTEGER NOT NULL DEFAULT 1,
            llm_free   INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE user_status (
            scope_type TEXT    NOT NULL CHECK(scope_type IN ('group','global')),
            scope_id   INTEGER NOT NULL,
            user_id    INTEGER NOT NULL,
            status     TEXT    NOT NULL DEFAULT 'blocked',
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (scope_type, scope_id, user_id)
        );
        CREATE INDEX idx_user_status_scope ON user_status (scope_type, scope_id);",
    )
}

// ── 工具函数 ──────────────────────────────────────────────────────────────────

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 将 `Scope` 转换为 DB 存储格式。
/// 返回 `(scope_type TEXT, scope_type u8 for cache, scope_id)`。
fn scope_parts(scope: &Scope) -> (&'static str, u8, i64) {
    match scope {
        Scope::Group(gid)   => ("group",  1, *gid),
        Scope::Private(_)   => ("global", 0, 0),   // Private → 按全局处理
    }
}
