pub mod cache;
mod model;
mod seed;
mod traits;

pub use model::*;
pub use seed::seed_from_history;
pub use traits::MessagePool;

// ── 类型别名 & 工厂函数 ────────────────────────────────────────────────────────

/// 消息池的具体实现类型（当前固定为 MemoryPool）。
pub type Pool = cache::MemoryPool;

/// 统一的消息池创建入口，在 `main.rs` 中调用。
/// 当前：MemoryPool
pub async fn create_pool(cfg: &PoolConfig) -> anyhow::Result<std::sync::Arc<Pool>> {
    Ok(cache::MemoryPool::new(cfg))
}
