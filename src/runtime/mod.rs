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

use std::collections::HashMap;
use std::sync::Arc;

/// Runtime 初始化摘要
#[derive(Default)]
pub struct RuntimeInitSummary {
    /// 已初始化的模块列表
    pub modules: Vec<String>,
    /// 模块详细信息（key: 模块名, value: 详细描述）
    pub details: HashMap<String, String>,
}

/// Runtime 层模块初始化函数。
/// 根据启用的 features 初始化对应的 runtime 模块，并注册到 App。
pub async fn init(app: &mut crate::kernel::app::App) -> anyhow::Result<RuntimeInitSummary> {


    let mut summary = RuntimeInitSummary::default();

    // ── 配置加载 ──────────────────────────────────────────────────────────
    #[cfg(feature = "runtime-config")]
    {
        config::init()?;
        summary.modules.push("config".to_string());
    }

    #[cfg(feature = "runtime-time")]
    {
        time::init();
        let offset = time::offset_hours();
        summary.modules.push("time".to_string());
        summary.details.insert("time".to_string(), format!("UTC{:+}", offset));
    }

    // ── 日志系统 ──────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-logger", feature = "runtime-config"))]
    {
        let log_cfg: logger::LogConfig = config::section("log");
        let _log_guard = logger::init(&log_cfg);
        summary.modules.push("logger".to_string());
    }

    // ── LLM 客户端 ────────────────────────────────────────────────────────
    #[cfg(feature = "runtime-llm")]
    {
        llm::init();
        summary.modules.push("llm".to_string());
        #[cfg(feature = "runtime-config")]
        {
            let llm_cfg: llm::LlmConfig = config::section("llm");
            summary.details.insert("llm".to_string(), llm_cfg.model.clone());
        }
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
        summary.modules.push("api".to_string());
        summary.details.insert("api".to_string(), napcat.url.clone());
    }

    // ── WebSocket 管理器 ──────────────────────────────────────────────────
    #[cfg(feature = "runtime-ws")]
    {
        let ws = ws::WsManager::new();
        app.set_ws(ws);
        summary.modules.push("ws".to_string());
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

        let group_count = access.enabled_groups().len();
        app.set_access(access);
        summary.modules.push("permission".to_string());
        summary.details.insert("permission".to_string(), format!("{} 个群", group_count));
    }

    // ── 消息池 ────────────────────────────────────────────────────────────
    #[cfg(all(feature = "runtime-pool", feature = "runtime-config"))]
    {
        let pool_cfg: pool::PoolConfig = config::section("pool");
        let pool = pool::create_pool(&pool_cfg).await?;
        app.set_pool(pool);
        summary.modules.push("pool".to_string());
        summary.details.insert("pool".to_string(), "预热中...".to_string());
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
        }
    }

    Ok(summary)
}
