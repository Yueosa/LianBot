// ── 历史消息查询接口 ───────────────────────────────────────────────────────────
//
// NapCat /get_group_msg_history 请求体：
//
// ```json
// {
//   "group_id": 123456789,
//   "count": 3000,
//   "message_seq": null,
//   "reverseOrder": false
// }
// ```
//
// 响应：{ "data": { "messages": [...] } }

use anyhow::Result;

use super::ApiClient;

impl ApiClient {
    /// 获取群历史消息（分页版本，可指定 message_seq 作为向前翻页起点）
    pub async fn get_group_msg_history_paged(
        &self,
        group_id: i64,
        count: u32,
        message_seq: Option<i64>,
    ) -> Result<Vec<serde_json::Value>> {
        let payload = serde_json::json!({
            "group_id": group_id,
            "count": count,
            "message_seq": message_seq,
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
