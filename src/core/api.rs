use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use tracing::{debug, warn};

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

    async fn post<P: Serialize + ?Sized>(
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

    // ── 发送消息 ──────────────────────────────────────────────────────────────

    /// 发送纯文字到群
    pub async fn send_text(&self, group_id: i64, text: &str) -> Result<()> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "message": [{"type": "text", "data": {"text": text}}]
        });
        self.post("/send_group_msg", &payload).await?;
        Ok(())
    }

    /// 发送图片到群（file 可为 URL 或 `base64://...`）
    pub async fn send_image(&self, group_id: i64, file: &str) -> Result<()> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "message": [{"type": "image", "data": {"file": file}}]
        });
        self.post("/send_group_msg", &payload).await?;
        Ok(())
    }

    /// 发送文字 + 图片到群（同一条消息，先文字后图片）
    pub async fn send_text_image(
        &self,
        group_id: i64,
        text: &str,
        file: &str,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "message": [
                {"type": "text",  "data": {"text": text}},
                {"type": "image", "data": {"file": file}}
            ]
        });
        self.post("/send_group_msg", &payload).await?;
        Ok(())
    }

    /// 发送完全自定义的消息段列表（当前未被命令用到，保留供未来）
    #[allow(dead_code)]
    pub async fn send_segments(
        &self,
        group_id: i64,
        segments: Vec<serde_json::Value>,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "message": segments
        });
        self.post("/send_group_msg", &payload).await?;
        Ok(())
    }

    // ── 获取消息历史 ──────────────────────────────────────────────────────────

    /// 获取群历史消息，返回原始 JSON 数组（每条消息均保留完整结构）
    pub async fn get_group_msg_history(
        &self,
        group_id: i64,
        count: u32,
    ) -> Result<Vec<serde_json::Value>> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "count": count,
            "reverseOrder": false
        });
        let resp = self.post("/get_group_msg_history", &payload).await?;
        let messages = resp["data"]["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(messages)
    }
}
