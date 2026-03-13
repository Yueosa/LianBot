//! 统一 Webhook 基础设施（runtime 级别）。
//!
//! 提供：
//!   - HMAC-SHA256 签名验证（`verify`）
//!   - 推送类型和消息构造辅助（`types`）
//!
//! 所有 Webhook Service（GitHub、YiBan 等）共享此模块，
//! 消除验签和消息构造的重复代码。

mod verify;
mod types;

pub use verify::verify_hmac_sha256;
pub use types::{PendingOrigin, build_notification};
