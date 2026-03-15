//! 易班签到服务
//!
//! 组件职责：
//!   - `register()` 向 App 注册 `/webhook/yiban` 路由和后台推送服务
//!   - `webhook_handler` 验证 HMAC 签名、解析 payload、发送到 channel
//!   - `YiBanService` 消费 channel → 按 targets 匹配 + pending 回源 → 群消息推送
//!   - 提供 `trigger_sign` / `get_status` 供指令调用 LianSign HTTP API
//!   - 提供 `bridge()` 供命令层设置 pending origin

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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

use crate::logic::yiban::{YiBanConfig, YiBanReport, format_report};
use crate::runtime::api::{ApiClient, MsgTarget};
use crate::runtime::permission::Scope;
use crate::runtime::webhook::{PendingOrigin, build_notification, verify_hmac_sha256};

use super::BotService;

// ── Bridge（命令层 ↔ 服务层通信） ─────────────────────────────────────────────

static BRIDGE: OnceLock<YiBanBridge> = OnceLock::new();

/// 命令层通过 `bridge()` 获取此对象，在触发签到前设置 pending origin。
pub struct YiBanBridge {
    pending: Arc<Mutex<Option<PendingOrigin>>>,
}

impl YiBanBridge {
    /// 记录触发来源，Webhook 回调时会额外推送到此 scope。
    pub fn set_origin(&self, scope: Scope) {
        *self.pending.lock().unwrap() = Some(PendingOrigin::new(scope));
    }
}

/// 获取 bridge 引用。若 yiban 服务未注册则返回 None。
pub fn bridge() -> Option<&'static YiBanBridge> {
    BRIDGE.get()
}

// ── 自注册入口 ────────────────────────────────────────────────────────────────

/// 注册易班签到 Webhook 路由和后台推送服务。
/// targets 为空时跳过（路由不注册）。
pub fn register(app: &mut crate::kernel::app::App) {
    let cfg = crate::logic::config::section::<YiBanConfig>("yiban");

    if cfg.targets.is_empty() {
        info!("[yiban] targets 未配置，/webhook/yiban 路由已禁用");
        return;
    }

    let pending: Arc<Mutex<Option<PendingOrigin>>> = Arc::new(Mutex::new(None));

    // 初始化 bridge（供命令层使用）
    let _ = BRIDGE.set(YiBanBridge { pending: pending.clone() });

    let (tx, rx) = mpsc::channel::<YiBanReport>(16);
    let secret = cfg.secret.clone();
    app.spawn(YiBanService::new(
        rx,
        app.api.clone().expect("runtime-api 未初始化"),
        cfg,
        pending
    ).run());

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
    // 1. 验证 HMAC-SHA256 签名（空 secret 时跳过）
    let sig = headers
        .get("X-YiBan-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_hmac_sha256(&state.secret, &body, sig, true) {
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
    pending: Arc<Mutex<Option<PendingOrigin>>>,
}

impl YiBanService {
    pub fn new(
        rx: mpsc::Receiver<YiBanReport>,
        api: Arc<ApiClient>,
        cfg: YiBanConfig,
        pending: Arc<Mutex<Option<PendingOrigin>>>,
    ) -> Self {
        Self { rx, api, cfg, pending }
    }
}

impl BotService for YiBanService {
    fn name(&self) -> &'static str {
        "yiban"
    }

    async fn run(mut self) -> anyhow::Result<()> {
        info!(
            "[{}] 已启动，共 {} 条推送规则",
            self.name(),
            self.cfg.targets.len()
        );

        while let Some(report) = self.rx.recv().await {
            let text = format_report(&report);
            let user_names: Vec<&str> = report.users.iter().map(|u| u.name.as_str()).collect();

            // 按 targets 匹配，按 group 聚合 at 列表（HashMap 天然去重）
            let mut target_map: HashMap<i64, Vec<i64>> = HashMap::new();
            for target in &self.cfg.targets {
                if target.matches_any(&user_names) {
                    target_map
                        .entry(target.group)
                        .or_default()
                        .extend(&target.at);
                }
            }

            // 检查 pending origin（命令触发回源）
            let origin = if self.cfg.reply_origin {
                self.pending.lock().unwrap().take()
            } else {
                // 关闭回源时丢弃 pending
                self.pending.lock().unwrap().take();
                None
            };
            if let Some(ref p) = origin {
                if !p.expired() {
                    if let Scope::Group(gid) = p.scope {
                        // 群回源直接合入 target_map，已存在则不重复
                        target_map.entry(gid).or_default();
                    }
                }
            }

            if target_map.is_empty() && origin.is_none() {
                info!("[yiban] 无匹配推送目标，跳过");
                continue;
            }

            // 推送到所有匹配群
            for (group_id, at_list) in &target_map {
                let segments = build_notification(&text, at_list);
                if let Err(e) = self
                    .api
                    .send_segments(MsgTarget::Group(*group_id), segments)
                    .await
                {
                    warn!("[yiban] 推送群 {group_id} 失败: {e:#}");
                } else {
                    info!("[yiban] 推送群 {group_id} 成功");
                }
            }

            // 私聊回源（如果 origin 是私聊且未过期）
            if let Some(ref p) = origin {
                if !p.expired() {
                    if let Scope::Private(uid) = p.scope {
                        if let Err(e) = self.api.send_msg(MsgTarget::Private(uid), &text).await {
                            warn!("[yiban] 回源私聊 {uid} 失败: {e:#}");
                        }
                    }
                }
            }
        }

        warn!("[{}] channel 已关闭，服务退出", self.name());
        Ok(())
    }
}

// ── 主动调用 LianSign HTTP API ────────────────────────────────────────────────

/// 触发签到（全部用户或指定用户名），返回触发结果消息
pub async fn trigger_sign(cfg: &YiBanConfig, name: Option<&str>) -> String {
    if cfg.api_url.is_empty() {
        return "未配置 api_url，无法调用签到服务".into();
    }
    let url = match name {
        Some(n) => format!("{}/sign/{}", cfg.api_url.trim_end_matches('/'), n),
        None => format!("{}/sign", cfg.api_url.trim_end_matches('/')),
    };
    let client = reqwest::Client::new();
    let mut req = client.post(&url);
    if !cfg.api_token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", cfg.api_token));
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                "签到已触发，稍后将收到结果通知".into()
            } else {
                format!("签到触发失败 (HTTP {}): {}", status.as_u16(), body)
            }
        }
        Err(e) => format!("无法连接签到服务: {e}"),
    }
}

/// 查询最近一次签到状态
pub async fn get_status(cfg: &YiBanConfig) -> String {
    if cfg.api_url.is_empty() {
        return "未配置 api_url，无法查询签到服务".into();
    }
    let url = format!("{}/status", cfg.api_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if !cfg.api_token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", cfg.api_token));
    }
    match req.send().await {
        Ok(resp) => {
            let body = resp.text().await.unwrap_or_default();
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    if let Some(data) = v.get("data") {
                        if data.is_null() {
                            return "暂无签到记录".into();
                        }
                        match serde_json::from_value::<YiBanReport>(data.clone()) {
                            Ok(report) => format_report(&report),
                            Err(_) => format!("解析签到数据失败: {body}"),
                        }
                    } else {
                        v.get("msg")
                            .and_then(|m| m.as_str())
                            .unwrap_or(&body)
                            .to_string()
                    }
                }
                Err(_) => body,
            }
        }
        Err(e) => format!("无法连接签到服务: {e}"),
    }
}
