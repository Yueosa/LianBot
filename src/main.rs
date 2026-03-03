mod core;
mod commands;
mod plugins;

use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json,
};
#[cfg(feature = "core-ws")]
use axum::extract::WebSocketUpgrade;
#[cfg(feature = "core-ws")]
use axum::routing::get;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{
    core::{
        api::ApiClient,
        dispatcher::Dispatcher,
        registry::CommandRegistry,
        typ::OneBotEvent,
        ws::WsManager,
    },
};

// ── 共享状态 ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    dispatcher: Arc<Dispatcher>,
    ws: Arc<WsManager>,
    api: Arc<ApiClient>,
}

// ── 入口 ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 日志（RUST_LOG 环境变量控制级别，默认 info）
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 加载配置
    core::config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    core::plugin_config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    let cfg = core::config::Config::global();
    info!("配置加载成功");
    info!("  NapCat URL : {}", cfg.napcat.url);
    info!("  服务监听   : {}:{}", cfg.server.host, cfg.server.port);
    info!("  群白名单   : {:?}", cfg.bot.whitelist);

    // 构建共享资源
    let api = Arc::new(ApiClient::new(
        cfg.napcat.url.clone(),
        cfg.napcat.token.clone(),
    ));
    let ws = WsManager::new();
    let registry = Arc::new(CommandRegistry::default());
    let pool = core::pool::create_pool(&cfg.pool).await
        .map_err(|e| anyhow::anyhow!("消息池初始化失败: {e}"))?;
    let dispatcher = Arc::new(Dispatcher::new(cfg, api.clone(), ws.clone(), registry, pool));

    let state = AppState {
        dispatcher,
        ws: ws.clone(),
        api,
    };

    // 路由
    let app = Router::new()
        .route("/", post(onebot_handler));   // OneBot HTTP 反向代理上报
    #[cfg(feature = "core-ws")]
    let app = app.route("/wstalk", get(ws_handler)); // WebSocket 截图客户端
    let app = app.with_state(state);

    // 启动服务
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("LianBot 已启动，监听 {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

// ── HTTP 路由处理函数 ──────────────────────────────────────────────────────────

/// OneBot 事件上报入口（HTTP POST /）
async fn onebot_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 健壮反序列化：失败时打印警告但始终返回 200（避免 OneBot 重试风暴）
    match serde_json::from_value::<OneBotEvent>(body.clone()) {
        Ok(event) => {
            state.dispatcher.dispatch(event).await;
        }
        Err(e) => {
            tracing::warn!("事件反序列化失败: {e}\n原始数据: {body}");
        }
    }
    StatusCode::OK
}

/// WebSocket 截图客户端接入（GET /wstalk）
#[cfg(feature = "core-ws")]
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        state.ws.clone().handle_socket(socket, state.api.clone())
    })
}
