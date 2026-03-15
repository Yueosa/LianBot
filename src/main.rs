mod kernel;
mod runtime;

#[cfg(feature = "runtime-dispatcher")]
mod commands;

mod logic;
mod services;

// ── 入口 ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> { kernel::boot::run().await }
