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
    info!("  Bot 主人   : {}", bot_cfg.owner);
    #[cfg(feature = "core-db")]
    info!("  权限 DB   : {}", bot_cfg.db_path);

    // ── 基础设施 ──────────────────────────────────────────────────────────────
    let api = Arc::new(ApiClient::new(napcat.url.clone(), napcat.token.clone()));
    let ws = WsManager::new();
    let pool = crate::runtime::pool::create_pool(&pool_cfg)
        .await
        .context("消息池初始化失败")?;

    #[cfg(feature = "core-db")]
    let access = AccessControl::open(
        std::path::Path::new(&bot_cfg.db_path),
        &bot_cfg.initial_groups,
    )
    .await
    .context("权限 DB 初始化失败")?;

    #[cfg(not(feature = "core-db"))]
    let access = AccessControl::from_config(&bot_cfg.initial_groups, &bot_cfg.blacklist);

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
    let router = app.into_router();
    let addr = format!("{}:{}", kcfg.host, kcfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("LianBot 已启动，监听 {addr}");
    axum::serve(listener, router).await?;

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
