use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
#[cfg(feature = "core-ws")]
use axum::{extract::WebSocketUpgrade, routing::get};
use anyhow::Context as _;
use tracing::info;

use crate::kernel::app::App;
use crate::runtime::{
    permission::{AccessControl, BotConfig},
    api::{ApiClient, NapcatConfig},
    dispatcher::Dispatcher,
    logger::LogConfig,
    parser::ParserConfig,
    pool::PoolConfig,
    typ::OneBotEvent,
    ws::WsManager,
};

pub async fn run() -> anyhow::Result<()> {
    // ── 三层配置加载 ──────────────────────────────────────────────────────────
    crate::kernel::config::init()?;
    crate::runtime::config::init()?;
    crate::runtime::time::init();
    crate::logic::config::init()?;

    let kcfg = crate::kernel::config::KernelConfig::global();
    let napcat: NapcatConfig     = crate::runtime::config::section("napcat");
    let bot_cfg: BotConfig       = crate::runtime::config::section("bot");
    let pool_cfg: PoolConfig     = crate::runtime::config::section("pool");
    let log_cfg: LogConfig       = crate::runtime::config::section("log");
    let parser_cfg: ParserConfig = crate::runtime::config::section("parser");

    let _log_guard = crate::runtime::logger::init(&log_cfg);
    info!("配置加载成功");
    info!("  NapCat URL : {}", napcat.url);
    info!("  服务监听   : {}:{}", kcfg.host, kcfg.port);
    info!("  Bot QQ    : {}", bot_cfg.bot_id);
    info!("  Bot 主人   : {}", bot_cfg.owner);
    #[cfg(feature = "core-db")]
    info!("  权限 DB   : {}", bot_cfg.db_path);

    // ── 基础设施 ──────────────────────────────────────────────────────────────
    crate::runtime::llm::init();
    let api = Arc::new(ApiClient::new(napcat.url.clone(), napcat.token.clone()));
    let ws = WsManager::new();
    let pool = crate::runtime::pool::create_pool(&pool_cfg)
        .await
        .context("消息池初始化失败")?;

    #[cfg(feature = "core-db")]
    let access = AccessControl::open(
        std::path::Path::new(&bot_cfg.db_path),
        &bot_cfg.initial_groups,
        &bot_cfg.group_blacklist,
        &bot_cfg.private_blacklist,
    )
    .await
    .context("权限 DB 初始化失败")?;

    #[cfg(not(feature = "core-db"))]
    let access = AccessControl::from_config(
        &bot_cfg.initial_groups,
        &bot_cfg.group_blacklist,
        &bot_cfg.private_blacklist,
    );

    // 启动预热（后台拉取历史消息填充 pool）
    {
        let api = api.clone();
        let pool = pool.clone();
        let groups = access.enabled_groups();
        tokio::spawn(async move {
            crate::runtime::pool::seed_from_history(&api, &pool, groups).await;
        });
    }

    // ── 构建 App（各层自注册命令 / 路由 / 后台服务） ──────────────────────────
    let mut app = App::new(api.clone(), ws.clone(), Some(pool.clone()), access.clone());

    crate::commands::register(&mut app);
    crate::services::register(&mut app);

    // Dispatcher + OneBot 路由
    let registry = app.take_registry();
    let dispatcher = Arc::new(Dispatcher::new(
        bot_cfg.bot_id,
        bot_cfg.owner,
        parser_cfg.cmd_prefix,
        api.clone(),
        ws.clone(),
        registry,
        Some(pool),
        access,
    ));
    app.merge(
        Router::new()
            .route("/", post(onebot_handler))
            .with_state(dispatcher),
    );

    // WebSocket 路由
    #[cfg(feature = "core-ws")]
    {
        let ws_state = WsState { ws, api };
        app.merge(
            Router::new()
                .route("/wstalk", get(ws_handler))
                .with_state(ws_state),
        );
    }

    // ── 启动服务 ──────────────────────────────────────────────────────────────
    let (router, task_handles) = app.into_router();
    let addr = format!("{}:{}", kcfg.host, kcfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("LianBot 已启动，监听 {addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // ── 优雅关闭：等待后台任务完成（最多 5s）────────────────────────────
    if !task_handles.is_empty() {
        info!("等待 {} 个后台任务结束...", task_handles.len());
        let drain = futures::future::join_all(task_handles);
        if tokio::time::timeout(std::time::Duration::from_secs(5), drain).await.is_err() {
            tracing::warn!("后台任务未在 5s 内完成，强制退出");
        }
    }
    info!("LianBot 已关闭");

    Ok(())
}

// ── Axum Handlers ─────────────────────────────────────────────────────────────

async fn onebot_handler(
    State(dispatcher): State<Arc<Dispatcher>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    match serde_json::from_value::<OneBotEvent>(body.clone()) {
        Ok(event) => {
            dispatcher.dispatch(event).await;
        }
        Err(e) => {
            tracing::warn!("事件反序列化失败: {e}\n原始数据: {body}");
        }
    }
    StatusCode::OK
}

#[cfg(feature = "core-ws")]
#[derive(Clone)]
struct WsState {
    ws: Arc<WsManager>,
    api: Arc<ApiClient>,
}

#[cfg(feature = "core-ws")]
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<WsState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| state.ws.clone().handle_socket(socket, state.api.clone()))
}

// ── Shutdown Signal ─────────────────────────────────────────────────────────────

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("注册 SIGTERM 失败")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("\n收到 SIGINT，开始关闭..."),
        _ = terminate => info!("收到 SIGTERM，开始关闭..."),
    }
}
