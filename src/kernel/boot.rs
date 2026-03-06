use std::sync::Arc;

use anyhow::Context;
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

use axum::body::Bytes;
use axum::http::HeaderMap;
use tokio::sync::mpsc;

use crate::{
    runtime::{
        permission::PermissionStore,
        api::ApiClient,
        dispatcher::Dispatcher,
        pool::{MessagePool, Pool, PoolMessage},
        registry::CommandRegistry,
        typ::OneBotEvent,
        ws::WsManager,
    },
    services::{
        BotService, ServiceContext,
        github::{GitHubEvent, GitHubService, verify_signature},
        scheduler::SchedulerService,
    },
};

#[derive(Clone)]
struct AppState {
    dispatcher: Arc<Dispatcher>,
    ws: Arc<WsManager>,
    api: Arc<ApiClient>,
    github_tx: Option<mpsc::Sender<GitHubEvent>>,
    github_secret: String,
}

pub async fn run() -> anyhow::Result<()> {
    crate::kernel::config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::runtime::plugin_config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    let cfg = crate::kernel::config::Config::global();

    let _log_guard = crate::runtime::logger::init(&cfg.log);
    info!("配置加载成功");
    info!("  NapCat URL : {}", cfg.napcat.url);
    info!("  服务监听   : {}:{}", cfg.server.host, cfg.server.port);
    info!("  Bot 主人   : {}", cfg.bot.owner);

    let api = Arc::new(ApiClient::new(cfg.napcat.url.clone(), cfg.napcat.token.clone()));
    let ws = WsManager::new();
    let registry = Arc::new(CommandRegistry::default());
    let pool = crate::runtime::pool::create_pool(&cfg.pool)
        .await
        .map_err(|e| anyhow::anyhow!("消息池初始化失败: {e}"))?;

    let perm = PermissionStore::open(
        std::path::Path::new(&cfg.bot.db_path),
        &cfg.bot.initial_groups,
    )
    .await
    .map_err(|e| anyhow::anyhow!("权限 DB 初始化失败: {e}"))?;

    {
        let api = api.clone();
        let pool = pool.clone();
        let groups = perm.enabled_groups();
        tokio::spawn(async move {
            seed_pool_for_whitelist(api, pool, groups).await;
        });
    }

    let dispatcher = Arc::new(Dispatcher::new(cfg, api.clone(), ws.clone(), registry, pool.clone(), perm.clone()));

    // ── 后台 Service ──────────────────────────────────────────────────────────
    let svc_ctx = ServiceContext { api: api.clone(), perm, pool: pool.clone(), config: cfg };
    tokio::spawn(SchedulerService::new(svc_ctx.clone()).run());

    // GitHub Webhook Service
    let gh_cfg = crate::runtime::plugin_config::PluginConfig::global()
        .get_section::<crate::services::github::GitHubConfig>("github");
    let github_secret = gh_cfg.secret.clone();
    let github_tx = if github_secret.is_empty() {
        info!("[github] secret 未配置，/webhook/github 路由已禁用");
        None
    } else {
        let (tx, rx) = mpsc::channel::<GitHubEvent>(64);
        tokio::spawn(GitHubService::new(rx, svc_ctx.clone(), gh_cfg).run());
        Some(tx)
    };

    let state = AppState {
        dispatcher,
        ws: ws.clone(),
        api,
        github_tx,
        github_secret,
    };

    let app = Router::new()
        .route("/", post(onebot_handler))
        .route("/webhook/github", post(github_webhook_handler));
    #[cfg(feature = "core-ws")]
    let app = app.route("/wstalk", get(ws_handler));
    let app = app.with_state(state);

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("LianBot 已启动，监听 {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn seed_pool_for_whitelist(api: Arc<ApiClient>, pool: Arc<Pool>, whitelist: Vec<i64>) {
    if whitelist.is_empty() {
        info!("[pool-seed] 无已开启的群，跳过启动预热");
        return;
    }

    info!("[pool-seed] 启动预热开始：{} 个群", whitelist.len());
    let mut total_seeded = 0usize;

    for group_id in whitelist {
        match seed_one_group(&api, &pool, group_id).await {
            Ok(n) => {
                total_seeded += n;
                info!("[pool-seed] 群 {} 预热完成：{} 条", group_id, n);
            }
            Err(e) => {
                tracing::warn!("[pool-seed] 群 {} 预热失败: {e:#}", group_id);
            }
        }
    }

    info!("[pool-seed] 启动预热结束：累计 {} 条", total_seeded);
}

async fn seed_one_group(api: &ApiClient, pool: &Arc<Pool>, group_id: i64) -> anyhow::Result<usize> {
    let raw = api
        .get_group_msg_history_paged(group_id, 3000, None)
        .await
        .with_context(|| format!("拉取群 {} 历史消息失败", group_id))?;

    let mut seeded = 0usize;
    for value in raw {
        if let Some(msg) = PoolMessage::from_api_value(&value, group_id) {
            pool.push(msg).await;
            seeded += 1;
        }
    }

    Ok(seeded)
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

async fn github_webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 路由禁用（secret 未配置）
    let Some(tx) = &state.github_tx else {
        return StatusCode::NOT_FOUND;
    };

    // 1. 验证 HMAC-SHA256 签名
    let sig = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(&state.github_secret, &body, sig) {
        tracing::warn!("[github] 签名验证失败，已拒绝请求");
        return StatusCode::UNAUTHORIZED;
    }

    // 2. 解析事件类型
    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    // 3. 解析 payload
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[github] payload JSON 解析失败: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    let repo = payload["repository"]["full_name"]
        .as_str()
        .unwrap_or("unknown/unknown")
        .to_string();
    let sender = payload["sender"]["login"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let evt = GitHubEvent { event_type, repo, sender, payload };
    if tx.send(evt).await.is_err() {
        tracing::warn!("[github] GitHubService channel 已关闭");
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
