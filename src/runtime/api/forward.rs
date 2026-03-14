// ── 合并转发消息收发 ───────────────────────────────────────────────────────────
//
// NapCat /get_forward_msg 请求体：
//   { "message_id": "xxx" }   或   { "id": "xxx" }
//
// 响应：{ "data": { "messages": [ { "sender": {..}, "time": .., "message": [..] }, ... ] } }
//
// NapCat /send_forward_msg 请求体：
//   {
//     "message_type": "group" | "private",
//     "group_id" / "user_id": ...,
//     "messages": [ { "type": "node", "data": { "user_id": "..", "nickname": "..", "content": [..] } }, ... ]
//   }

use anyhow::{Context as _, Result};
use tracing::{debug, warn};

use super::{ApiClient, MsgTarget};
use crate::runtime::typ::MessageSegment;

/// 合并转发中的单条消息节点（支持嵌套）。
///
/// **预留功能**：用于解析和理解收到的合并转发消息。
/// 当前仅实现了发送合并转发（`send_forward_msg`），接收解析功能（`get_forward_msg`）
/// 已实现但暂未使用，将在机器人需要阅读合并转发内容时启用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ForwardNode {
    pub sender_id: i64,
    pub nickname: String,
    pub time: i64,
    /// 该节点的消息段
    pub segments: Vec<MessageSegment>,
    /// 嵌套的合并转发（递归展开后填充）
    pub nested: Vec<ForwardNode>,
}

/// 递归深度上限，防止无限嵌套
#[allow(dead_code)]
const MAX_DEPTH: u8 = 5;

impl ApiClient {
    /// 获取合并转发消息内容，**递归**展开嵌套转发。
    ///
    /// **预留功能**：用于解析收到的合并转发消息，支持递归展开嵌套结构。
    /// 将在机器人需要理解合并转发内容时启用（如 AI 对话上下文、日报生成等场景）。
    ///
    /// **协议限制**：OneBot v11 协议的 forward segment 不包含 resId 字段，
    /// 但 NapCat 需要 resId 来解析多层嵌套的合并转发。因此当前实现只能解析一层，
    /// 无法从接收到的消息事件中直接提取 resId 进行递归展开。
    /// 此限制需等待协议扩展或 NapCat 提供替代方案。
    ///
    /// `id` 为 forward segment 中的 id 字段（resId）。
    /// 返回顶层节点列表，每个 `ForwardNode` 的 `nested` 字段包含递归子节点。
    #[allow(dead_code)]
    pub async fn get_forward_msg(&self, id: &str) -> Result<Vec<ForwardNode>> {
        self.get_forward_msg_inner(id, 0).await
    }

    #[allow(dead_code)]
    fn get_forward_msg_inner<'a>(
        &'a self,
        id: &'a str,
        depth: u8,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<ForwardNode>>> + Send + 'a>> {
        Box::pin(async move {
        if depth >= MAX_DEPTH {
            warn!("[forward] 递归深度达到 {MAX_DEPTH}，停止展开 id={id}");
            return Ok(vec![]);
        }

        debug!("[forward] get_forward_msg id={id} depth={depth}");
        let payload = serde_json::json!({ "id": id });
        let resp = self.post_with_timeout("/get_forward_msg", &payload, Some(self.history_timeout())).await
            .context("get_forward_msg: API 调用失败")?;

        let messages = resp
            .pointer("/data/messages")
            .or_else(|| resp.get("messages"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut nodes = Vec::with_capacity(messages.len());
        for msg in &messages {
            let sender_id = msg
                .pointer("/sender/user_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let nickname = msg
                .pointer("/sender/nickname")
                .and_then(|v| v.as_str())
                .unwrap_or("未知")
                .to_string();
            let time = msg.get("time").and_then(|v| v.as_i64()).unwrap_or(0);

            let segments: Vec<MessageSegment> = msg
                .get("message")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            // 递归展开嵌套的合并转发
            let mut nested = Vec::new();
            for seg in &segments {
                if let Some(fwd_id) = seg.forward_id() {
                    match self.get_forward_msg_inner(fwd_id, depth + 1).await {
                        Ok(children) => nested.extend(children),
                        Err(e) => warn!("[forward] 展开嵌套转发失败: {e}"),
                    }
                }
            }

            nodes.push(ForwardNode {
                sender_id,
                nickname,
                time,
                segments,
                nested,
            });
        }

        Ok(nodes)
        })
    }

    /// 发送合并转发消息到任意目标。
    ///
    /// `nodes` 是 `MessageSegment::node(...)` 构成的数组。
    /// 可选参数：`source`（来源标题）、`summary`（底部摘要）、`prompt`（外显文本）。
    pub async fn send_forward_msg(
        &self,
        target: MsgTarget,
        nodes: Vec<MessageSegment>,
        source: Option<&str>,
        summary: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<()> {
        debug!("[forward] send_forward_msg {} nodes → {target:?}", nodes.len());
        let mut payload = target.into_payload();
        payload["messages"] = serde_json::to_value(&nodes)?;
        if let Some(s) = source  { payload["source"]  = serde_json::json!(s); }
        if let Some(s) = summary { payload["summary"] = serde_json::json!(s); }
        if let Some(s) = prompt  { payload["prompt"]   = serde_json::json!(s); }
        self.post("/send_forward_msg", &payload).await?;
        Ok(())
    }
}
