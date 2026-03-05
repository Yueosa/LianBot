mod kernel;
mod runtime;
mod commands;
mod logic;
#[cfg(feature = "core-db")]
mod db;
mod permission;

// ── 入口 ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> { kernel::boot::run().await }
