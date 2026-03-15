#[cfg(feature = "runtime-config")]
pub mod config;

#[cfg(feature = "runtime-api")]
pub mod api;

#[cfg(feature = "runtime-typ")]
pub mod typ;

#[cfg(feature = "runtime-dispatcher")]
pub mod dispatcher;

#[cfg(feature = "runtime-llm")]
pub mod llm;

#[cfg(feature = "runtime-logger")]
pub mod logger;

#[cfg(feature = "runtime-time")]
pub mod time;

#[cfg(feature = "runtime-parser")]
pub mod parser;

#[cfg(feature = "runtime-permission")]
pub mod permission;

#[cfg(feature = "runtime-pool")]
pub mod pool;

#[cfg(feature = "runtime-registry")]
pub mod registry;

#[cfg(feature = "runtime-ws")]
pub mod ws;

#[cfg(feature = "core-webhook")]
pub mod webhook;

#[cfg(feature = "core-db")]
pub mod db;

// ── Runtime 层模块初始化 ──────────────────────────────────────────────────────

/// Runtime 层模块初始化函数。
/// 根据启用的 features 初始化对应的 runtime 模块，并注册到 App。
pub async fn init(app: &mut crate::kernel::app::App) -> anyhow::Result<()> {
    use std::sync::Arc;
    use tracing::info;

    // ── 配置加载 ──────────────────────────────────────────────────────────
    #[cfg(feature = "runtime-config")]
    {
        config::init()?;
        info!("[runtime] config 已初始化");
    }

    #[cfg(feature = "runtime-time")]
    {
        time::init();
        info!("[runtime] time 已初始化");
    }

    // ── 日志系统 ──────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-logger", feature = "runtime-config"))]
    {
        let log_cfg: logger::LogConfig = config::section("log");
        let _log_guard = logger::init(&log_cfg);
        info!("[runtime] logger 已初始化");
    }

    // ── LLM 客户端 ────────────────────────────────────────────────────────
    #[cfg(feature = "runtime-llm")]
    {
        llm::init();
        info!("[runtime] llm 已初始化");
    }

    // ── API 客户端 ────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-api", feature = "runtime-config"))]
    {
        let napcat: api::NapcatConfig = config::section("napcat");
        let api = Arc::new(api::ApiClient::with_config(
            napcat.url.clone(),
            napcat.token.clone(),
            napcat.timeout_secs,
            napcat.history_timeout_secs,
        ));
        app.set_api(api);
        info!("[runtime] api 已初始化: {}", napcat.url);
    }

    // ── WebSocket 管理器 ──────────────────────────────────────────────────
    #[cfg(feature = "runtime-ws")]
    {
        let ws = ws::WsManager::new();
        app.set_ws(ws);
        info!("[runtime] ws 已初始化");
    }

    // ── 权限控制 ──────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-permission", feature = "runtime-config"))]
    {
        let bot_cfg: permission::BotConfig = config::section("bot");

        #[cfg(feature = "core-db")]
        let access = permission::AccessControl::open(
            std::path::Path::new(&bot_cfg.db_path),
            &bot_cfg.initial_groups,
            &bot_cfg.group_blacklist,
            &bot_cfg.private_blacklist,
        ).await?;

        #[cfg(not(feature = "core-db"))]
        let access = permission::AccessControl::from_config(
            &bot_cfg.initial_groups,
            &bot_cfg.group_blacklist,
            &bot_cfg.private_blacklist,
        );

        app.set_access(access);
        info!("[runtime] permission 已初始化");
    }

    // ── 消息池 ────────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-pool", feature = "runtime-config"))]
    {
        let pool_cfg: pool::PoolConfig = config::section("pool");
        let pool = pool::create_pool(&pool_cfg).await?;
        app.set_pool(pool);
        info!("[runtime] pool 已初始化");
    }

    // ── 消息池预热 ────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-pool", feature = "runtime-api", feature = "runtime-permission"))]
    {
        if let (Some(api), Some(pool), Some(access)) = (&app.api, &app.pool, &app.access) {
            let api_clone = api.clone();
            let pool_clone = pool.clone();
            let groups = access.enabled_groups();
            tokio::spawn(async move {
                pool::seed_from_history(&api_clone, &pool_clone, groups).await;
            });
            info!("[runtime] pool 预热已启动");
        }
    }

    Ok(())
}
