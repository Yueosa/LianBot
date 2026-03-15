//! GitHub Webhook 通知服务
//!
//! 组件职责：
//!   - `register()` 向 App 注册 `/webhook/github` 路由和后台推送服务
//!   - `webhook_handler` 验证 HMAC 签名、解析 payload、发送到 channel
//!   - `GitHubService` 消费 channel、订阅匹配、群消息推送
//!
//! 业务逻辑（配置模型、验签、格式化）位于 `logic::github`。

use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::logic::github::{GitHubConfig, GitHubEvent, format_event};
use crate::runtime::api::{ApiClient, MsgTarget};
use crate::runtime::webhook::{build_notification, verify_hmac_sha256};

use super::BotService;

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 注册 GitHub Webhook 路由和后台推送服务。
/// secret 为空时跳过（路由不注册，返回 404）。
pub fn register(app: &mut crate::kernel::app::App) {
    let gh_cfg = crate::logic::config::section::<GitHubConfig>("github");

    if gh_cfg.secret.is_empty() {
        info!("[github] secret 未配置，/webhook/github 路由已禁用");
        return;
    }

    let (tx, rx) = mpsc::channel::<GitHubEvent>(64);
    let secret = gh_cfg.secret.clone();
    app.spawn(GitHubService::new(
        rx,
        app.api.clone().expect("runtime-api 未初始化"),
        gh_cfg
    ).run());

    app.merge(
        Router::new()
            .route("/webhook/github", post(webhook_handler))
            .with_state(WebhookState { tx, secret }),
    );
}

// ── Webhook Axum Handler ──────────────────────────────────────────────────────

#[derive(Clone)]
struct WebhookState {
    tx: mpsc::Sender<GitHubEvent>,
    secret: String,
}

async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. 验证 HMAC-SHA256 签名
    let sig = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_hmac_sha256(&state.secret, &body, sig, false) {
        warn!("[github] 签名验证失败，已拒绝请求");
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
            warn!("[github] payload JSON 解析失败: {e}");
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

    info!("[github] 收到 webhook: {event_type} / {repo} by {sender}");
    let evt = GitHubEvent { event_type, repo, sender, payload };
    if state.tx.send(evt).await.is_err() {
        warn!("[github] GitHubService channel 已关闭");
    }

    StatusCode::OK
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct GitHubService {
    rx: mpsc::Receiver<GitHubEvent>,
    api: Arc<ApiClient>,
    cfg: GitHubConfig,
}

impl GitHubService {
    pub fn new(rx: mpsc::Receiver<GitHubEvent>, api: Arc<ApiClient>, cfg: GitHubConfig) -> Self {
        Self { rx, api, cfg }
    }
}

impl BotService for GitHubService {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn run(mut self) -> anyhow::Result<()> {
        while let Some(evt) = self.rx.recv().await {
            let Some(text) = format_event(&evt, &self.cfg) else {
                continue;
            };

            // 找出所有匹配该事件的订阅
            let targets: Vec<(i64, Vec<i64>)> = self
                .cfg
                .subscriptions
                .iter()
                .filter(|s| s.matches(&evt.repo, &evt.event_type))
                .map(|s| (s.group, s.at.clone()))
                .collect();

            if targets.is_empty() {
                info!("[github] 事件 {} / {} 无匹配订阅，跳过", evt.event_type, evt.repo);
                continue;
            }

            info!("[github] 推送 {} / {} → {} 个群", evt.event_type, evt.repo, targets.len());
            for (group_id, at_list) in targets {
                let segments = build_notification(&text, &at_list);

                if let Err(e) = self
                    .api
                    .send_segments(MsgTarget::Group(group_id), segments)
                    .await
                {
                    warn!("[github] 推送群 {group_id} 失败: {e:#}");
                }
            }
        }

        warn!("[{}] channel 已关闭，服务退出", self.name());
        Ok(())
    }
}
