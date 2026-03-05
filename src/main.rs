mod kernel;
mod runtime;
mod commands;
mod logic;

// ── 入口 ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> { kernel::boot::run().await }
