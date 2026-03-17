// ── runtime::api ───────────────────────────────────────────────────────────────
//
// NapCat / OneBot v11 HTTP API 客户端。
//
// 依赖：runtime::typ（MessageSegment 构造器 + Serialize）
//
// 子模块：
//   send_msg  — /send_msg 发消息接口
//   history   — /get_group_msg_history 等查询接口
//   forward   — 合并转发消息收发（get_forward_msg / send_forward_msg）
//   image     — /get_image 图片下载接口

mod send_msg;
mod history;
mod forward;
mod image;


use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ── NapCat 连接配置 ────────────────────────────────────────────────────────────

/// runtime.toml `[napcat]` 段。
#[derive(Debug, Deserialize)]
pub struct NapcatConfig {
    /// NapCat/go-cqhttp HTTP API 地址，例如 "http://127.0.0.1:3000"
    #[serde(default = "NapcatConfig::default_url")]
    pub url: String,
    /// Bearer Token（可选）
    pub token: Option<String>,
    /// 普通请求超时秒数，默认 30s
    #[serde(default = "NapcatConfig::default_timeout")]
    pub timeout_secs: u64,
    /// 历史消息/转发展开等慢请求超时秒数，默认 120s
    #[serde(default = "NapcatConfig::default_history_timeout")]
    pub history_timeout_secs: u64,
}

impl NapcatConfig {
    fn default_url() -> String { "http://127.0.0.1:3000".to_string() }
    fn default_timeout() -> u64 { 30 }
    fn default_history_timeout() -> u64 { 120 }
}

impl Default for NapcatConfig {
    fn default() -> Self {
        Self {
            url: Self::default_url(),
            token: None,
            timeout_secs: Self::default_timeout(),
            history_timeout_secs: Self::default_history_timeout(),
        }
    }
}

// ── 消息发送目标 ───────────────────────────────────────────────────────────────

/// 消息发送目标，供 send_msg / send_image_to 等通用接口使用。
#[derive(Debug, Clone, Copy)]
pub enum MsgTarget {
    /// 群聊，携带 group_id
    Group(i64),
    /// 私聊，携带对方 QQ 号
    Private(i64),
}

impl MsgTarget {
    /// 生成 /send_msg 所需的 base payload（含 message_type + group_id 或 user_id）
    pub(crate) fn into_payload(self) -> serde_json::Value {
        match self {
            MsgTarget::Group(id) => serde_json::json!({
                "message_type": "group",
                "group_id": id,
            }),
            MsgTarget::Private(id) => serde_json::json!({
                "message_type": "private",
                "user_id": id,
            }),
        }
    }
}

impl From<crate::runtime::permission::Scope> for MsgTarget {
    fn from(scope: crate::runtime::permission::Scope) -> Self {
        match scope {
            crate::runtime::permission::Scope::Group(gid) => MsgTarget::Group(gid),
            crate::runtime::permission::Scope::Private(uid) => MsgTarget::Private(uid),
        }
    }
}

// ── API 客户端 ─────────────────────────────────────────────────────────────────
//
// 封装所有对 NapCat/go-cqhttp HTTP API 的调用。
// 使用 runtime::http 提供的全局 HTTP 客户端，共享连接池。

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    token: Option<String>,
    /// 普通请求超时时长
    timeout: std::time::Duration,
    /// 历史消息拉取等慢请求的超时时长
    history_timeout: std::time::Duration,
}

impl ApiClient {
    /// 创建客户端。`base_url` 例如 `"http://127.0.0.1:3000"`
    #[allow(dead_code)]
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self::with_config(base_url, token, 30, 120)
    }

    pub fn with_config(
        base_url: impl Into<String>,
        token: Option<String>,
        timeout_secs: u64,
        history_timeout_secs: u64,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
            timeout: std::time::Duration::from_secs(timeout_secs),
            history_timeout: std::time::Duration::from_secs(history_timeout_secs),
        }
    }

    // ── 底层请求 ──────────────────────────────────────────────────────────────

    pub(crate) async fn post<P: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        payload: &P,
    ) -> Result<serde_json::Value> {
        self.post_with_timeout(endpoint, payload, None).await
    }

    /// 使用自定义超时发送 POST 请求，用于历史消息拉取等慢请求场景。
    pub(crate) async fn post_with_timeout<P: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        payload: &P,
        timeout: Option<std::time::Duration>,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/{}", self.base_url, endpoint.trim_start_matches('/'));
        debug!("API POST {url}");

        #[cfg(feature = "runtime-http")]
        let client = crate::runtime::http::client();
        #[cfg(not(feature = "runtime-http"))]
        let client = {
            static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .timeout(self.timeout)
                    .build()
                    .expect("构建 HTTP 客户端失败")
            })
        };

        let mut req = client.post(&url).json(payload);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        // 设置超时：优先使用传入的超时，否则使用配置的超时
        let timeout = timeout.unwrap_or(self.timeout);
        req = req.timeout(timeout);

        let resp = req
            .send()
            .await
            .with_context(|| format!("发送请求失败: {url}"))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .with_context(|| "响应 JSON 解析失败")?;

        if !status.is_success() {
            warn!("API 返回非 2xx 状态 {status}: {body}");
        }

        Ok(body)
    }

    /// 获取历史消息超时配置，供子模块使用
    pub(crate) fn history_timeout(&self) -> std::time::Duration {
        self.history_timeout
    }
}
