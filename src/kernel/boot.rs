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
use tracing::info;

use crate::kernel::app::App;

#[cfg(all(feature = "runtime-dispatcher", feature = "runtime-config"))]
use crate::runtime::{
    dispatcher::Dispatcher,
    permission::BotConfig,
    parser::ParserConfig,
};

#[cfg(feature = "runtime-typ")]
use crate::runtime::typ::OneBotEvent;

#[cfg(feature = "runtime-ws")]
use crate::runtime::ws::WsManager;

pub async fn run() -> anyhow::Result<()> {
    // ── 配置加载（kernel 层基础配置） ─────────────────────────────────────
    crate::kernel::config::init()?;
    crate::logic::config::init()?;

    let kcfg = crate::kernel::config::KernelConfig::global();

    info!("配置加载成功");
    info!("  服务监听   : {}:{}", kcfg.host, kcfg.port);

    // ── 创建空的 App 管理器 ──────────────────────────────────────────────
    let mut app = App::new();

    // ── Runtime 层自己初始化并注册模块 ────────────────────────────────────
    crate::runtime::init(&mut app).await?;

    // ── Commands 层注册命令 ───────────────────────────────────────────────
    crate::commands::register(&mut app);

    // ── Services 层注册服务和路由 ─────────────────────────────────────────
    crate::services::register(&mut app);

    // ── Dispatcher + OneBot 路由 ──────────────────────────────────────────
    #[cfg(feature = "runtime-dispatcher")]
    {
        let registry = app.take_registry();

        // 从 app 中获取已初始化的模块
        let api = app.api.clone().expect("runtime-api 未初始化");
        let access = app.access.clone().expect("runtime-permission 未初始化");
        let pool = app.pool.clone();

        #[cfg(feature = "runtime-config")]
        let bot_cfg: BotConfig = crate::runtime::config::section("bot");
        #[cfg(feature = "runtime-config")]
        let parser_cfg: ParserConfig = crate::runtime::config::section("parser");

        #[cfg(all(feature = "runtime-config", feature = "runtime-ws"))]
        let dispatcher = Arc::new(Dispatcher::new(
            bot_cfg.bot_id,
            bot_cfg.owner,
            parser_cfg.cmd_prefix,
            api.clone(),
            app.ws.clone(),
            registry,
            pool,
            access,
        ));

        #[cfg(all(feature = "runtime-config", not(feature = "runtime-ws")))]
        let dispatcher = Arc::new(Dispatcher::new(
            bot_cfg.bot_id,
            bot_cfg.owner,
            parser_cfg.cmd_prefix,
            api.clone(),
            registry,
            pool,
            access,
        ));

        #[cfg(feature = "runtime-config")]
        app.merge(
            Router::new()
                .route("/", post(onebot_handler))
                .with_state(dispatcher),
        );
    }

    // ── WebSocket 路由 ────────────────────────────────────────────────────
    #[cfg(feature = "core-ws")]
    {
        let ws = app.ws.clone().expect("runtime-ws 未初始化（core-ws 依赖 runtime-ws）");
        let api = app.api.clone().expect("runtime-api 未初始化（core-ws 依赖 runtime-api）");
        app.merge(
            Router::new()
                .route("/wstalk", get(ws_handler))
                .with_state(WsState { ws, api }),
        );
    }

    // ── 启动服务器 ────────────────────────────────────────────────────────
    let (router, _handles) = app.into_router();
    let listener = tokio::net::TcpListener::bind((kcfg.host.as_str(), kcfg.port)).await?;

    info!("🚀 LianBot 启动成功: http://{}:{}", kcfg.host, kcfg.port);
    axum::serve(listener, router).await?;

    Ok(())
}

// ── Axum Handlers ─────────────────────────────────────────────────────────────

#[cfg(feature = "runtime-dispatcher")]
async fn onebot_handler(
    State(dispatcher): State<Arc<Dispatcher>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    #[cfg(feature = "runtime-typ")]
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
    ws.on_upgrade(move |socket| async move {
        state.ws.handle_connection(socket, &state.api).await;
    })
}
