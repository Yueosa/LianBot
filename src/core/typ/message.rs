// 这个文件为协议类型层，提供内置 accessor，部分方法当前未在命令中用到但保留供未来使用。
#![allow(dead_code)]

use serde::Deserialize;

// ── 消息段 ────────────────────────────────────────────────────────────────────
//
// OneBot v11 中每条消息由多个 MessageSegment 组成：
//   [{"type": "text", "data": {"text": "hello"}}, ...]
//
// 为了保持健壮性（未来可能出现新的 type），使用含 `type` 字符串 + `data: Value`
// 的平坦结构，而非强类型枚举（避免反序列化失败）。
// 常用 segment 提供便捷 accessor。

#[derive(Debug, Deserialize, Clone)]
pub struct MessageSegment {
    /// 消息类型：text / image / at / face / record / video / ...
    #[serde(rename = "type")]
    pub seg_type: String,

    /// 原始 data 字段，保留完整信息
    pub data: serde_json::Value,
}

impl MessageSegment {
    // ── 文本 ─────────────────────────────────────────────────────────────────

    pub fn is_text(&self) -> bool { self.seg_type == "text" }

    /// 提取纯文本内容
    pub fn as_text(&self) -> Option<&str> {
        if self.is_text() {
            self.data.get("text").and_then(|v| v.as_str())
        } else {
            None
        }
    }

    // ── 图片 ─────────────────────────────────────────────────────────────────

    pub fn is_image(&self) -> bool { self.seg_type == "image" }

    /// 提取图片 URL（接收方推送的 url 字段）
    pub fn image_url(&self) -> Option<&str> {
        if self.is_image() {
            self.data.get("url").and_then(|v| v.as_str())
        } else {
            None
        }
    }

    /// 提取图片 file 字段（可能是 base64:// 或路径）
    pub fn image_file(&self) -> Option<&str> {
        if self.is_image() {
            self.data.get("file").and_then(|v| v.as_str())
        } else {
            None
        }
    }

    // ── @某人 ────────────────────────────────────────────────────────────────

    pub fn is_at(&self) -> bool { self.seg_type == "at" }

    /// 提取 @ 目标 QQ 号字符串（"all" 表示 @全体成员）
    pub fn at_qq(&self) -> Option<&str> {
        if self.is_at() {
            self.data.get("qq").and_then(|v| v.as_str())
        } else {
            None
        }
    }

    // ── 构造工具 ─────────────────────────────────────────────────────────────

    /// 构造纯文本段
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            seg_type: "text".into(),
            data: serde_json::json!({"text": content.into()}),
        }
    }

    /// 构造图片段（URL 或 base64://）
    pub fn image(file: impl Into<String>) -> Self {
        Self {
            seg_type: "image".into(),
            data: serde_json::json!({"file": file.into()}),
        }
    }

    /// 将自身序列化为发送载荷所需的 JSON Value
    pub fn to_send_value(&self) -> serde_json::Value {
        serde_json::json!({
            "type": self.seg_type,
            "data": self.data
        })
    }
}
