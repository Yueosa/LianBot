//! 通用 SQLite 句柄。不包含任何业务表定义。
//!
//! 业务方（permission、订阅等）各自持有一个 [`SqliteDb`]，
//! 通过 [`SqliteDb::call`] 直接执行 SQL，自行管理建表与迁移逻辑。

mod sqlite;

pub use sqlite::SqliteDb;

/// 迁移函数类型。
///
/// 对应一个 schema 版本（index 0 = v1，index 1 = v2，…）。  
/// 框架负责事务包裹和版本号写入，迁移函数**只做 DDL / DML，不开启事务**。
pub type MigrationFn = fn(&rusqlite::Connection) -> rusqlite::Result<()>;
