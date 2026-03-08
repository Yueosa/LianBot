mod format;
mod model;
mod pager;
mod parser;

pub use format::format_for_llm;
#[allow(unused_imports)]
pub use model::{ChatMessage, FetchResult, FetchSource, GapLevel, GapWarning};
pub use pager::fetch;

/// 解析时间字符串如 "30m" / "2h" / "1d"，返回对应的秒数
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "m" => Some(num * 60),
        "h" => Some(num * 3600),
        "d" => Some(num * 86400),
        _ => None,
    }
}
