//! 全局时区工具。
//!
//! 从 `runtime.toml` 的 `[time]` 段读取 `timezone` 偏移量（UTC 偏移小时数），
//! 提供统一的时间获取接口，消除各模块硬编码 UTC+8 的问题。

use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, TimeZone, Timelike};
use serde::Deserialize;

static TZ_OFFSET: OnceLock<i32> = OnceLock::new();

/// runtime.toml `[time]` 段。
#[derive(Debug, Deserialize)]
struct TimeConfig {
    /// UTC 偏移小时数，默认 8（中国标准时间 CST = UTC+8）
    #[serde(default = "TimeConfig::default_tz")]
    timezone: i32,
}

impl TimeConfig {
    fn default_tz() -> i32 { 8 }
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self { timezone: Self::default_tz() }
    }
}

/// 初始化时区（boot 中调用一次，须在 `runtime::config::init` 之后）。
pub fn init() {
    let cfg: TimeConfig = crate::runtime::config::section("time");
    TZ_OFFSET.set(cfg.timezone).expect("runtime::time 已重复初始化");
}

/// 配置的 UTC 偏移小时数。
pub fn offset_hours() -> i32 {
    *TZ_OFFSET.get().expect("runtime::time 未初始化")
}

/// 构造 `chrono::FixedOffset`。
pub fn fixed_offset() -> FixedOffset {
    FixedOffset::east_opt(offset_hours() * 3600).expect("无效时区偏移")
}

/// 配置时区的当前时间。
pub fn now() -> DateTime<FixedOffset> {
    chrono::Utc::now().with_timezone(&fixed_offset())
}

/// 将 Unix 时间戳转为配置时区的 `DateTime`。
pub fn from_timestamp(ts: i64) -> Option<DateTime<FixedOffset>> {
    fixed_offset().timestamp_opt(ts, 0).single()
}

/// 将 Unix 时间戳提取为配置时区的小时 (0–23)。
pub fn hour_of_day(ts: i64) -> u32 {
    from_timestamp(ts).map(|dt| dt.hour()).unwrap_or(0)
}

/// 构造 `time::UtcOffset`（供 logger 等使用 `time` crate 的模块）。
pub fn utc_offset() -> ::time::UtcOffset {
    ::time::UtcOffset::from_hms(offset_hours() as i8, 0, 0).expect("无效时区偏移")
}

/// 获取当前 Unix 时间戳（秒）。
///
/// 系统时间异常时返回 0（1970-01-01），这可能导致消息被立即淘汰等问题。
/// TODO: 考虑改为 panic 或返回 Result，避免静默失败。
pub fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
