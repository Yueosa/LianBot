
#[cfg(feature = "core-ws")]
use axum::{extract::WebSocketUpgrade, routing::get};

#[cfg(feature = "runtime-dispatcher")]
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};

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

#[cfg(all(feature = "core-ws", feature = "runtime-api"))]
use crate::runtime::api::ApiClient;

pub async fn run() -> anyhow::Result<()> {
    // ── 启动横幅 ──────────────────────────────────────────────────────────
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🤖 LianBot v{} 正在启动...", env!("CARGO_PKG_VERSION"));
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ── 配置加载 ──────────────────────────────────────────────────────────
    info!("┌─ 配置加载");
    crate::kernel::config::init()?;
    crate::logic::config::init()?;

    let kcfg = crate::kernel::config::KernelConfig::global();
    info!("│  服务监听: {}:{}", kcfg.host, kcfg.port);

    #[cfg(all(feature = "runtime-config", feature = "runtime-permission"))]
    {
        let bot_cfg: BotConfig = crate::runtime::config::section("bot");
        info!("│  Bot QQ: {}", bot_cfg.bot_id);
        info!("│  Bot 主人: {}", bot_cfg.owner);
    }

    info!("└─ ✓ 配置加载完成");

    // ── Runtime 层初始化 ──────────────────────────────────────────────────
    info!("┌─ Runtime 模块初始化");
    let mut app = App::new();
    let runtime_summary = crate::runtime::init(&mut app).await?;

    for module in &runtime_summary.modules {
        let detail = runtime_summary.details.get(module)
            .map(|s| format!(" ({})", s))
            .unwrap_or_default();
        info!("│  ✓ {}{}", module, detail);
    }
    info!("└─ ✓ Runtime 初始化完成 ({} 个模块)", runtime_summary.modules.len());

    // ── Commands 层注册 ───────────────────────────────────────────────────
    #[cfg(feature = "runtime-dispatcher")]
    {
        info!("┌─ 命令注册");
        let cmd_summary = crate::commands::register(&mut app);
        info!("│  ✓ {}", cmd_summary.names.join(", "));
        info!("└─ ✓ 已注册 {} 个命令", cmd_summary.count);
    }

    // ── Services 层注册 ───────────────────────────────────────────────────
    info!("┌─ 服务注册");
    let svc_summary = crate::services::register(&mut app);
    for detail in &svc_summary.details {
        info!("│  ✓ {}", detail);
    }
    info!("└─ ✓ 已注册 {} 个服务", svc_summary.count);

    // ── Dispatcher + OneBot 路由 ──────────────────────────────────────────
    #[cfg(feature = "runtime-dispatcher")]
    {
        let registry = app.take_registry();

        // 从 app 中获取已初始化的模块
        let api = app.api.clone().expect("runtime-api 未初始化");
        let access = app.access.clone().expect("runtime-permission 未初始化");
        #[cfg(feature = "runtime-pool")]
        let pool = app.pool.clone();

        #[cfg(feature = "runtime-config")]
        let bot_cfg: BotConfig = crate::runtime::config::section("bot");
        #[cfg(feature = "runtime-config")]
        let parser_cfg: ParserConfig = crate::runtime::config::section("parser");

        #[cfg(all(feature = "runtime-config", feature = "runtime-ws", feature = "runtime-pool"))]
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

        #[cfg(all(feature = "runtime-config", not(feature = "runtime-ws"), feature = "runtime-pool"))]
        let dispatcher = Arc::new(Dispatcher::new(
            bot_cfg.bot_id,
            bot_cfg.owner,
            parser_cfg.cmd_prefix,
            api.clone(),
            registry,
            pool,
            access,
        ));

        #[cfg(all(feature = "runtime-config", feature = "runtime-ws", not(feature = "runtime-pool")))]
        let dispatcher = Arc::new(Dispatcher::new(
            bot_cfg.bot_id,
            bot_cfg.owner,
            parser_cfg.cmd_prefix,
            api.clone(),
            app.ws.clone(),
            registry,
            access,
        ));

        #[cfg(all(feature = "runtime-config", not(feature = "runtime-ws"), not(feature = "runtime-pool")))]
        let dispatcher = Arc::new(Dispatcher::new(
            bot_cfg.bot_id,
            bot_cfg.owner,
            parser_cfg.cmd_prefix,
            api.clone(),
            registry,
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

    // ── 启动成功 ──────────────────────────────────────────────────────────
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🚀 LianBot 启动成功");
    info!("   HTTP: http://{}:{}", kcfg.host, kcfg.port);
    #[cfg(feature = "core-ws")]
    info!("   WebSocket: ws://{}:{}/wstalk", kcfg.host, kcfg.port);
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

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
        state.ws.handle_socket(socket, state.api).await;
    })
}
