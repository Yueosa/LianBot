use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
#[cfg(feature = "core-ws")]
use axum::extract::WebSocketUpgrade;
#[cfg(feature = "core-ws")]
use axum::routing::get;
use tracing::info;

use crate::runtime::{
    api::ApiClient,
    dispatcher::Dispatcher,
    registry::CommandRegistry,
    typ::OneBotEvent,
    ws::WsManager,
};

#[derive(Clone)]
struct AppState {
    dispatcher: Arc<Dispatcher>,
    ws: Arc<WsManager>,
    api: Arc<ApiClient>,
}

pub async fn run() -> anyhow::Result<()> {
    crate::kernel::config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::runtime::plugin_config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    let cfg = crate::kernel::config::Config::global();

    let _log_guard = crate::runtime::logger::init(&cfg.log);
    info!("配置加载成功");
    info!("  NapCat URL : {}", cfg.napcat.url);
    info!("  服务监听   : {}:{}", cfg.server.host, cfg.server.port);
    info!("  群白名单   : {:?}", cfg.bot.whitelist);

    let api = Arc::new(ApiClient::new(cfg.napcat.url.clone(), cfg.napcat.token.clone()));
    let ws = WsManager::new();
    let registry = Arc::new(CommandRegistry::default());
    let pool = crate::runtime::pool::create_pool(&cfg.pool)
        .await
        .map_err(|e| anyhow::anyhow!("消息池初始化失败: {e}"))?;
    let dispatcher = Arc::new(Dispatcher::new(cfg, api.clone(), ws.clone(), registry, pool));

    let state = AppState {
        dispatcher,
        ws: ws.clone(),
        api,
    };

    let app = Router::new().route("/", post(onebot_handler));
    #[cfg(feature = "core-ws")]
    let app = app.route("/wstalk", get(ws_handler));
    let app = app.with_state(state);

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("LianBot 已启动，监听 {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn onebot_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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

#[cfg(feature = "core-ws")]
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| state.ws.clone().handle_socket(socket, state.api.clone()))
}
