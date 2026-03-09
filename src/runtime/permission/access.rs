// ══════════════════════════════════════════════════════════════════════════════
//  AccessControl — 群/用户准入控制
//
//  两种实现，编译时二选一：
//    core-db ON  → DB 版（SQLite 持久化 + 内存缓存）
//    core-db OFF → Config 版（从 config.toml 静态读取）
//
//  黑名单分两级：
//    group_blacklist   — 群聊黑名单，Bot 忽略该用户在任何群中的消息
//    private_blacklist — 私聊黑名单，Bot 忽略该用户的私聊消息
//  DB 版额外支持运行时 per-scope block/unblock（admin 命令动态管理）。
// ══════════════════════════════════════════════════════════════════════════════

// ── DB 版 ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "core-db")]
mod inner {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::{Arc, RwLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::Context as _;
    use tracing::info;

    use crate::runtime::db::SqliteDb;
    use super::super::model::Scope;

    #[derive(Debug, Clone)]
    struct GroupPolicy {
        enabled: bool,
        /// 预留：LLM 自由发言开关
        #[allow(dead_code)]
        llm_free: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct BlockKey {
        /// 1 = group, 2 = private
        scope_type: u8,
        scope_id: i64,
        user_id: i64,
    }

    #[derive(Debug, Default)]
    struct PermState {
        groups: HashMap<i64, GroupPolicy>,
        blocked: HashSet<BlockKey>,
    }

    /// DB 版准入控制：内存缓存（热路径）+ SQLite 持久化（写穿）。
    /// 配置黑名单作为不可变底层，DB 动态 block 作为叠加层。
    pub struct AccessControl {
        db: SqliteDb,
        state: RwLock<PermState>,
        /// 配置文件群聊黑名单（不可变，启动时加载）
        cfg_group_blocked: HashSet<i64>,
        /// 配置文件私聊黑名单（不可变，启动时加载）
        cfg_private_blocked: HashSet<i64>,
    }

    impl AccessControl {
        /// 打开权限数据库，加载缓存，并将 `initial_groups` 中尚不存在的群写入。
        pub async fn open(
            path: &Path,
            initial_groups: &[i64],
            group_blacklist: &[i64],
            private_blacklist: &[i64],
        ) -> anyhow::Result<Arc<Self>> {
            let path_buf = path.to_path_buf();
            let db = tokio::task::spawn_blocking(move || {
                SqliteDb::open(&path_buf, &[migration_v1])
            })
            .await
            .context("db open task panicked")?
            .with_context(|| format!("open {}", path.display()))?;

            // 导入初始群
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

            // 全量 load groups
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
                                    enabled: row.get::<_, i64>(1)? != 0,
                                    llm_free: row.get::<_, i64>(2)? != 0,
                                },
                            ))
                        })?
                        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
                    Ok(rows)
                })
                .await
                .context("load group_policy")?;

            // 全量 load blocked users
            let blocked: HashSet<BlockKey> = db
                .call(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT scope_type, scope_id, user_id FROM user_status WHERE status = 'blocked'",
                    )?;
                    let rows = stmt
                        .query_map([], |row| {
                            let st: String = row.get(0)?;
                            Ok(BlockKey {
                                scope_type: match st.as_str() {
                                    "group" => 1,
                                    "private" => 2,
                                    _ => 0,
                                },
                                scope_id: row.get(1)?,
                                user_id: row.get(2)?,
                            })
                        })?
                        .collect::<rusqlite::Result<HashSet<_>>>()?;
                    Ok(rows)
                })
                .await
                .context("load user_status")?;

            info!(
                "[access] 已加载 {} 个群策略，{} 条 DB 黑名单，配置黑名单: 群 {} / 私聊 {}",
                groups.len(),
                blocked.len(),
                group_blacklist.len(),
                private_blacklist.len(),
            );

            Ok(Arc::new(Self {
                db,
                state: RwLock::new(PermState { groups, blocked }),
                cfg_group_blocked: group_blacklist.iter().copied().collect(),
                cfg_private_blocked: private_blacklist.iter().copied().collect(),
            }))
        }

        // ── 读操作（纯内存） ──────────────────────────────────────────────────

        pub fn is_group_enabled(&self, group_id: i64) -> bool {
            self.state
                .read()
                .expect("perm RwLock poisoned")
                .groups
                .get(&group_id)
                .map(|p| p.enabled)
                .unwrap_or(false)
        }

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

        /// 判断用户是否被拉黑（配置黑名单 + DB 动态黑名单两层叠加）。
        pub fn is_user_blocked(&self, user_id: i64, scope: &Scope) -> bool {
            match scope {
                Scope::Group(gid) => {
                    // 配置群聊黑名单（全群生效）
                    if self.cfg_group_blocked.contains(&user_id) {
                        return true;
                    }
                    // DB per-group 黑名单
                    let state = self.state.read().expect("perm RwLock poisoned");
                    state.blocked.contains(&BlockKey {
                        scope_type: 1,
                        scope_id: *gid,
                        user_id,
                    })
                }
                Scope::Private(_) => {
                    // 配置私聊黑名单
                    if self.cfg_private_blocked.contains(&user_id) {
                        return true;
                    }
                    // DB 私聊黑名单
                    let state = self.state.read().expect("perm RwLock poisoned");
                    state.blocked.contains(&BlockKey {
                        scope_type: 2,
                        scope_id: 0,
                        user_id,
                    })
                }
            }
        }

        // ── 写操作（内存 + 写穿 DB） ─────────────────────────────────────────

        pub async fn block_user(&self, scope: &Scope, user_id: i64) -> anyhow::Result<()> {
            let (scope_type_str, scope_type_u8, scope_id) = scope_parts(scope);
            let key = BlockKey { scope_type: scope_type_u8, scope_id, user_id };

            self.state
                .write()
                .expect("perm RwLock poisoned")
                .blocked
                .insert(key);

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

        pub async fn unblock_user(&self, scope: &Scope, user_id: i64) -> anyhow::Result<()> {
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

        /// 启用群：写入 group_policy + 内存缓存。
        pub async fn enable_group(&self, group_id: i64) -> anyhow::Result<()> {
            self.state
                .write()
                .expect("perm RwLock poisoned")
                .groups
                .insert(group_id, GroupPolicy { enabled: true, llm_free: false });

            let now = unix_now();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "INSERT INTO group_policy (group_id, enabled, llm_free, updated_at) \
                         VALUES (?1, 1, 0, ?2) \
                         ON CONFLICT(group_id) DO UPDATE SET enabled = 1, updated_at = ?2",
                        rusqlite::params![group_id, now],
                    )?;
                    Ok(())
                })
                .await
                .context("enable_group: db write")
        }

        /// 禁用群：置 enabled=false + 内存缓存。
        pub async fn disable_group(&self, group_id: i64) -> anyhow::Result<()> {
            if let Some(policy) = self.state
                .write()
                .expect("perm RwLock poisoned")
                .groups
                .get_mut(&group_id)
            {
                policy.enabled = false;
            }

            let now = unix_now();
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE group_policy SET enabled = 0, updated_at = ?2 WHERE group_id = ?1",
                        rusqlite::params![group_id, now],
                    )?;
                    Ok(())
                })
                .await
                .context("disable_group: db write")
        }
    }

    // ── 迁移、工具 ───────────────────────────────────────────────────────────

    fn migration_v1(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE group_policy (
                group_id   INTEGER PRIMARY KEY,
                enabled    INTEGER NOT NULL DEFAULT 1,
                llm_free   INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE user_status (
                scope_type TEXT    NOT NULL CHECK(scope_type IN ('group','private')),
                scope_id   INTEGER NOT NULL,
                user_id    INTEGER NOT NULL,
                status     TEXT    NOT NULL DEFAULT 'blocked',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (scope_type, scope_id, user_id)
            );
            CREATE INDEX idx_user_status_scope ON user_status (scope_type, scope_id);",
        )
    }

    fn unix_now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn scope_parts(scope: &Scope) -> (&'static str, u8, i64) {
        match scope {
            Scope::Group(gid) => ("group", 1, *gid),
            Scope::Private(_) => ("private", 2, 0),
        }
    }
}

// ── Config 版（无 DB fallback） ───────────────────────────────────────────────

#[cfg(not(feature = "core-db"))]
mod inner {
    use std::collections::HashSet;
    use std::sync::Arc;

    use super::super::model::Scope;

    /// Config 版准入控制：静态白名单 + 双级黑名单，来自 runtime.toml。
    /// 无持久化，block/unblock 只在内存生效（重启丢失）。
    pub struct AccessControl {
        allowed_groups: std::sync::RwLock<HashSet<i64>>,
        group_blocked: std::sync::RwLock<HashSet<i64>>,
        private_blocked: std::sync::RwLock<HashSet<i64>>,
    }

    impl AccessControl {
        /// 从 runtime.toml 字段构造。
        pub fn from_config(
            initial_groups: &[i64],
            group_blacklist: &[i64],
            private_blacklist: &[i64],
        ) -> Arc<Self> {
            Arc::new(Self {
                allowed_groups: std::sync::RwLock::new(initial_groups.iter().copied().collect()),
                group_blocked: std::sync::RwLock::new(group_blacklist.iter().copied().collect()),
                private_blocked: std::sync::RwLock::new(private_blacklist.iter().copied().collect()),
            })
        }

        pub fn is_group_enabled(&self, group_id: i64) -> bool {
            self.allowed_groups
                .read()
                .expect("groups RwLock poisoned")
                .contains(&group_id)
        }

        pub fn enabled_groups(&self) -> Vec<i64> {
            self.allowed_groups
                .read()
                .expect("groups RwLock poisoned")
                .iter()
                .copied()
                .collect()
        }

        pub fn is_user_blocked(&self, user_id: i64, scope: &Scope) -> bool {
            match scope {
                Scope::Group(_) => {
                    self.group_blocked
                        .read()
                        .expect("group_blocked RwLock poisoned")
                        .contains(&user_id)
                }
                Scope::Private(_) => {
                    self.private_blocked
                        .read()
                        .expect("private_blocked RwLock poisoned")
                        .contains(&user_id)
                }
            }
        }

        /// 内存 block（无 DB 时重启丢失）
        pub async fn block_user(&self, scope: &Scope, user_id: i64) -> anyhow::Result<()> {
            match scope {
                Scope::Group(_) => {
                    self.group_blocked
                        .write()
                        .expect("group_blocked RwLock poisoned")
                        .insert(user_id);
                }
                Scope::Private(_) => {
                    self.private_blocked
                        .write()
                        .expect("private_blocked RwLock poisoned")
                        .insert(user_id);
                }
            }
            Ok(())
        }

        /// 内存 unblock（无 DB 时重启丢失）
        pub async fn unblock_user(&self, scope: &Scope, user_id: i64) -> anyhow::Result<()> {
            match scope {
                Scope::Group(_) => {
                    self.group_blocked
                        .write()
                        .expect("group_blocked RwLock poisoned")
                        .remove(&user_id);
                }
                Scope::Private(_) => {
                    self.private_blocked
                        .write()
                        .expect("private_blocked RwLock poisoned")
                        .remove(&user_id);
                }
            }
            Ok(())
        }

        /// 启用群（内存，无 DB 时重启丢失）
        pub async fn enable_group(&self, group_id: i64) -> anyhow::Result<()> {
            self.allowed_groups
                .write()
                .expect("groups RwLock poisoned")
                .insert(group_id);
            Ok(())
        }

        /// 禁用群（内存，无 DB 时重启丢失）
        pub async fn disable_group(&self, group_id: i64) -> anyhow::Result<()> {
            self.allowed_groups
                .write()
                .expect("groups RwLock poisoned")
                .remove(&group_id);
            Ok(())
        }
    }
}

// ── 统一 re-export ────────────────────────────────────────────────────────────

pub use inner::AccessControl;
