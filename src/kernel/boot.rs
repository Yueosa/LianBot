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
    logic::github::{GitHubConfig, GitHubEvent, verify_signature},
    runtime::{
        permission::{AccessControl, BotConfig},
        api::{ApiClient, NapcatConfig},
        dispatcher::Dispatcher,
        logger::LogConfig,
        parser::ParserConfig,
        pool::{MessagePool, Pool, PoolConfig, PoolMessage},
        registry::CommandRegistry,
        typ::OneBotEvent,
        ws::WsManager,
    },
    services::{
        BotService, ServiceContext,
        github::GitHubService,
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
    // ── 三层配置加载 ──────────────────────────────────────────────────────────
    crate::kernel::config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::runtime::config::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::runtime::logic_config::init().map_err(|e| anyhow::anyhow!("{e}"))?;

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

    let api = Arc::new(ApiClient::new(napcat.url.clone(), napcat.token.clone()));
    let ws = WsManager::new();
    let registry = Arc::new(CommandRegistry::default());
    let pool = crate::runtime::pool::create_pool(&pool_cfg)
        .await
        .map_err(|e| anyhow::anyhow!("消息池初始化失败: {e}"))?;

    #[cfg(feature = "core-db")]
    let access = AccessControl::open(
        std::path::Path::new(&bot_cfg.db_path),
        &bot_cfg.initial_groups,
    )
    .await
    .map_err(|e| anyhow::anyhow!("权限 DB 初始化失败: {e}"))?;

    #[cfg(not(feature = "core-db"))]
    let access = AccessControl::from_config(&bot_cfg.initial_groups, &bot_cfg.blacklist);

    {
        let api = api.clone();
        let pool = pool.clone();
        let groups = access.enabled_groups();
        tokio::spawn(async move {
            seed_pool_for_whitelist(api, pool, groups).await;
        });
    }

    let dispatcher = Arc::new(Dispatcher::new(
        bot_cfg.owner,
        parser_cfg.cmd_prefix,
        api.clone(),
        ws.clone(),
        registry,
        Some(pool.clone()),
        access.clone(),
    ));

    // ── 后台 Service ──────────────────────────────────────────────────────────
    let svc_ctx = ServiceContext { api: api.clone(), access, pool: Some(pool.clone()) };
    tokio::spawn(SchedulerService::new(svc_ctx.clone()).run());

    // GitHub Webhook Service
    let gh_cfg = crate::runtime::logic_config::section::<GitHubConfig>("github");
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

    let addr = format!("{}:{}", kcfg.host, kcfg.port);
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
