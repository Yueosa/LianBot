// ── /send_msg 接口 ─────────────────────────────────────────────────────────────
//
// NapCat /send_msg 请求体结构：
//
// ```json
// {
//   "message_type": "group" | "private",
//   "user_id": 114514,
//   "group_id": 123456789,
//   "message": [
//     { "type": "text",  "data": { "text": "hello" } },
//     { "type": "image", "data": { "file": "https://..." } }
//   ],
//   "auto_escape": false
// }
// ```
//
// 所有发送方法统一走 /send_msg，通过 MsgTarget 区分群聊/私聊。
// 消息段使用 typ::MessageSegment 构造器 + Serialize，避免手写 JSON。

use anyhow::Result;
use tracing::debug;

use super::{ApiClient, MsgTarget};
use crate::runtime::typ::MessageSegment;

impl ApiClient {
    // ── 通用发送 ──────────────────────────────────────────────────────────────

    /// 发送纯文字到任意目标（群聊或私聊）
    pub async fn send_msg(&self, target: MsgTarget, text: &str) -> Result<()> {
        self.send_segments(target, vec![MessageSegment::text(text)]).await
    }

    /// 发送图片到任意目标（file 可为 URL 或 `base64://...`）
    pub async fn send_image_to(&self, target: MsgTarget, file: &str) -> Result<()> {
        self.send_segments(target, vec![MessageSegment::image(file)]).await
    }

    /// 发送任意消息段列表到任意目标
    pub async fn send_segments(&self, target: MsgTarget, segments: Vec<MessageSegment>) -> Result<()> {
        let tag = segments.iter().map(|s| s.seg_type.as_str()).collect::<Vec<_>>().join("+");
        debug!("[api] send {tag} → {target:?}");
        let mut payload = target.into_payload();
        payload["message"] = serde_json::to_value(&segments)?;
        self.post("/send_msg", &payload).await?;
        Ok(())
    }

}
