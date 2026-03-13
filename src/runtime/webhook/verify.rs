//! 统一 HMAC-SHA256 Webhook 签名验证。
//!
//! 支持 `sha256=<hex>` 格式（GitHub / YiBan 通用）。

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 验证 `sha256=<hex>` 格式的 HMAC-SHA256 签名。
///
/// - `skip_empty`: 为 `true` 时，空 secret 视为验证通过（YiBan 行为）；
///   为 `false` 时，空 secret 一律拒绝（GitHub 行为）。
pub fn verify_hmac_sha256(secret: &str, body: &[u8], header: &str, skip_empty: bool) -> bool {
    if secret.is_empty() {
        return skip_empty;
    }
    let hex_part = match header.strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };
    let sig_bytes = match hex::decode(hex_part) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}
