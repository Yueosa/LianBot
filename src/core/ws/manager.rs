use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::core::api::ApiClient;

// 广播频道容量（最多缓存 N 条未消费消息）
const BROADCAST_CAPACITY: usize = 32;

// ── WsManager ─────────────────────────────────────────────────────────────────
//
// 负责：
//   1. 追踪在线连接数
//   2. 向所有客户端广播消息（stalk 请求等）
//   3. 接收客户端上报（截图数据），调用 API 发送到群

#[derive(Clone)]
pub struct WsManager {
    tx: broadcast::Sender<String>,
    conn_count: Arc<AtomicUsize>,
}

impl WsManager {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(Self {
            tx,
            conn_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// 是否有客户端在线
    pub async fn has_clients(&self) -> bool {
        self.conn_count.load(Ordering::Relaxed) > 0
    }

    /// 向所有已连接的客户端广播文本消息
    pub async fn broadcast(&self, msg: String) {
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(msg);
        }
    }

    /// 处理一条新的 WebSocket 连接（由 axum 路由调用）
    /// 使用 tokio::select! 在同一个 task 内交替处理：
    ///   - 广播消息 → 推给客户端
    ///   - 客户端消息 → 处理截图结果等
    pub async fn handle_socket(self: Arc<Self>, mut socket: WebSocket, api: Arc<ApiClient>) {
        self.conn_count.fetch_add(1, Ordering::Relaxed);
        info!("WebSocket 客户端已连接，当前连接数: {}", self.conn_count.load(Ordering::Relaxed));

        let mut broadcast_rx = self.tx.subscribe();

        loop {
            tokio::select! {
                // 广播 → 发给客户端
                result = broadcast_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if socket.send(Message::Text(msg.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("广播队列落后 {n} 条，部分消息被丢弃");
                        }
                        Err(_) => break,
                    }
                }
                // 客户端消息 → 处理
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            self.handle_client_message(text.to_string(), &api).await;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }
            }
        }

        self.conn_count.fetch_sub(1, Ordering::Relaxed);
        info!("WebSocket 客户端已断开，当前连接数: {}", self.conn_count.load(Ordering::Relaxed));
    }

    // ── 客户端消息协议 ────────────────────────────────────────────────────────
    //
    // 格式（文本帧）：
    //   stalk_result:<group_id>:<base64_image>   截图二进制（base64 编码）
    //   stalk_text:<group_id>:<text>             截图附带的文字信息
    //   heartbeat                                心跳（静默忽略）

    async fn handle_client_message(&self, text: String, api: &ApiClient) {
        if text == "heartbeat" {
            return;
        }

        if let Some(rest) = text.strip_prefix("stalk_result:") {
            if let Some((group_str, img_data)) = rest.split_once(':') {
                if let Ok(group_id) = group_str.parse::<i64>() {
                    // 兼容 data URL 格式 "data:image/jpeg;base64,<data>" 和裸 base64
                    let pure_b64 = img_data
                        .find(";base64,")
                        .map(|i| &img_data[i + 8..])
                        .unwrap_or(img_data);
                    let file = format!("base64://{pure_b64}");
                    if let Err(e) = api.send_image(group_id, &file).await {
                        warn!("发送截图失败: {e}");
                    }
                }
            }
            return;
        }

        if let Some(rest) = text.strip_prefix("stalk_text:") {
            if let Some((group_str, msg_text)) = rest.split_once(':') {
                if let Ok(group_id) = group_str.parse::<i64>() {
                    if let Err(e) = api.send_text(group_id, msg_text).await {
                        warn!("发送文字失败: {e}");
                    }
                }
            }
            return;
        }

        warn!("收到未知格式的 WS 消息: {text}");
    }
}
