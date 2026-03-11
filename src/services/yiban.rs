//! 易班签到 Webhook 通知服务
//!
//! 组件职责：
//!   - `register()` 向 App 注册 `/webhook/yiban` 路由和后台推送服务
//!   - `webhook_handler` 验证 HMAC 签名、解析 payload、发送到 channel
//!   - `YiBanService` 消费 channel、格式化消息、群消息推送

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

use crate::logic::yiban::{YiBanConfig, YiBanReport, format_report, verify_signature};
use crate::runtime::api::{ApiClient, MsgTarget};
use crate::runtime::typ::MessageSegment;

use super::BotService;

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 注册易班签到 Webhook 路由和后台推送服务。
/// group 为 0 时跳过（路由不注册）。
pub fn register(app: &mut crate::kernel::app::App) {
    let cfg = crate::logic::config::section::<YiBanConfig>("yiban");

    if cfg.group == 0 {
        info!("[yiban] group 未配置，/webhook/yiban 路由已禁用");
        return;
    }

    let (tx, rx) = mpsc::channel::<YiBanReport>(16);
    let secret = cfg.secret.clone();
    app.spawn(YiBanService::new(rx, app.api.clone(), cfg).run());

    app.merge(
        Router::new()
            .route("/webhook/yiban", post(webhook_handler))
            .with_state(WebhookState { tx, secret }),
    );
}

// ── Webhook Axum Handler ──────────────────────────────────────────────────────

#[derive(Clone)]
struct WebhookState {
    tx: mpsc::Sender<YiBanReport>,
    secret: String,
}

async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. 验证 HMAC-SHA256 签名
    let sig = headers
        .get("X-YiBan-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(&state.secret, &body, sig) {
        warn!("[yiban] 签名验证失败，已拒绝请求");
        return StatusCode::UNAUTHORIZED;
    }

    // 2. 解析 payload
    let report: YiBanReport = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!("[yiban] payload JSON 解析失败: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    info!(
        "[yiban] 收到签到报告: {} 个用户, 耗时 {}s",
        report.users.len(),
        report.elapsed
    );

    if state.tx.send(report).await.is_err() {
        warn!("[yiban] YiBanService channel 已关闭");
    }

    StatusCode::OK
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct YiBanService {
    rx: mpsc::Receiver<YiBanReport>,
    api: Arc<ApiClient>,
    cfg: YiBanConfig,
}

impl YiBanService {
    pub fn new(rx: mpsc::Receiver<YiBanReport>, api: Arc<ApiClient>, cfg: YiBanConfig) -> Self {
        Self { rx, api, cfg }
    }
}

impl BotService for YiBanService {
    fn name(&self) -> &'static str {
        "yiban"
    }

    async fn run(mut self) -> anyhow::Result<()> {
        info!("[{}] 已启动，推送目标群: {}", self.name(), self.cfg.group);

        while let Some(report) = self.rx.recv().await {
            let text = format_report(&report);

            // 构造消息段：@ 段 + 换行 + 文本
            let mut segments: Vec<MessageSegment> = self
                .cfg
                .at
                .iter()
                .map(|&qq| MessageSegment::at(qq))
                .collect();
            if !segments.is_empty() {
                segments.push(MessageSegment::text("\n"));
            }
            segments.push(MessageSegment::text(text.as_str()));

            if let Err(e) = self
                .api
                .send_segments(MsgTarget::Group(self.cfg.group), segments)
                .await
            {
                warn!("[yiban] 推送群 {} 失败: {e:#}", self.cfg.group);
            } else {
                info!("[yiban] 推送群 {} 成功", self.cfg.group);
            }
        }

        warn!("[{}] channel 已关闭，服务退出", self.name());
        Ok(())
    }
}
