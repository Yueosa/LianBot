// ── runtime::api ───────────────────────────────────────────────────────────────
//
// NapCat / OneBot v11 HTTP API 客户端。
//
// 依赖：runtime::typ（MessageSegment 构造器 + Serialize）
//
// 子模块：
//   send_msg  — /send_msg 发消息接口
//   history   — /get_group_msg_history 等查询接口

mod send_msg;
mod history;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use tracing::{debug, warn};

// ── re-export ─────────────────────────────────────────────────────────────────

pub use send_msg::*;
pub use history::*;

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

// ── API 客户端 ─────────────────────────────────────────────────────────────────
//
// 封装所有对 NapCat/go-cqhttp HTTP API 的调用。
// 内部持有 reqwest::Client（异步、连接池复用）。

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    /// 创建客户端。`base_url` 例如 `"http://127.0.0.1:3000"`
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            client: Client::new(),
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
        let url = format!("{}/{}", self.base_url, endpoint.trim_start_matches('/'));
        debug!("API POST {url}");

        let mut req = self.client.post(&url).json(payload);
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
