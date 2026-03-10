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

mod send_msg;
mod history;
mod forward;

pub use forward::ForwardNode;

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
}

impl NapcatConfig {
    fn default_url() -> String { "http://127.0.0.1:3000".to_string() }
}

impl Default for NapcatConfig {
    fn default() -> Self {
        Self { url: Self::default_url(), token: None }
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
// 内部持有 reqwest::Client（异步、连接池复用）。

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    /// 单独给历史拉取等慢请求使用，超时更长
    client_slow: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    /// 创建客户端。`base_url` 例如 `"http://127.0.0.1:3000"`
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("构建 HTTP 客户端失败");
        let client_slow = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("构建 HTTP 慢请求客户端失败");
        Self {
            client,
            client_slow,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
        }
    }

    // ── 底层请求 ──────────────────────────────────────────────────────────────

    pub(crate) async fn post<P: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        payload: &P,
    ) -> Result<serde_json::Value> {
        self.post_with(&self.client, endpoint, payload).await
    }

    /// 使用慢请求 client（120s 超时）发送 POST，用于历史消息拉取等场景。
    pub(crate) async fn post_slow<P: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        payload: &P,
    ) -> Result<serde_json::Value> {
        self.post_with(&self.client_slow, endpoint, payload).await
    }

    async fn post_with<P: Serialize + ?Sized>(
        &self,
        client: &Client,
        endpoint: &str,
        payload: &P,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/{}", self.base_url, endpoint.trim_start_matches('/'));
        debug!("API POST {url}");

        let mut req = client.post(&url).json(payload);
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

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
}
