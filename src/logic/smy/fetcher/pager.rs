use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{debug, warn};

use crate::runtime::{
    api::ApiClient,
    pool::{MessagePool, Pool, PoolMessage},
};

use super::format::detect_gap;
use super::model::{FetchResult, FetchSource};
use super::parser::{parse_raw_messages, pool_msg_to_chat};

/// 从消息池或 NapCat 拉取历史消息并结构化。
///
/// 读取策略（time-first）：
///   1. 优先从内存池读取（微秒级，无网络消耗）
///   2. pool 未完整覆盖 → 分页调用 NapCat API（count=5000, message_seq 回溯）
///   3. API 结果自动 back-seed 到 pool（下次直接命中）
pub async fn fetch(
    api: &ApiClient,
    pool: &Option<Arc<Pool>>,
    group_id: i64,
    time_window: Duration,
) -> Result<FetchResult> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - time_window.as_secs() as i64;

    // 优先尝试 pool 路径
    if let Some(pool) = pool {
        let pool_msgs = pool.range(group_id, cutoff, now).await;
        let oldest = pool.oldest_timestamp(group_id).await;
        if !pool_msgs.is_empty() && oldest.is_some_and(|t| t <= cutoff) {
            debug!(
                "[fetcher] 时间模式: pool完整覆盖 {} 条 (oldest={}, cutoff={})",
                pool_msgs.len(),
                oldest.unwrap(),
                cutoff
            );
            let mut messages = pool_msgs.iter().map(pool_msg_to_chat).collect::<Vec<_>>();
            messages.sort_by_key(|m| m.time);
            return Ok(FetchResult {
                gap: detect_gap(&messages),
                messages,
                source: FetchSource::Pool,
            });
        }
        debug!(
            "[fetcher] 时间模式: pool起点={:?} > cutoff={}, 回退 API 分页",
            oldest, cutoff
        );
    } else {
        debug!("[fetcher] 无消息池，直接走 API 分页");
    }

    let (raw, reached_cutoff, earliest_ts) = fetch_api_until_cutoff(api, group_id, cutoff).await?;
    if let Some(pool) = pool {
        back_seed_pool(pool, &raw, group_id, cutoff).await;
    }
    let mut messages = parse_raw_messages(&raw, Some(cutoff));
    messages.sort_by_key(|m| m.time);

    let source = if reached_cutoff {
        FetchSource::Api
    } else {
        warn!(
            "[fetcher] 服务端历史已穷尽但未覆盖请求窗口: earliest={:?}, cutoff={}, group={}",
            earliest_ts, cutoff, group_id
        );
        FetchSource::ApiExhausted
    };

    debug!(
        "[fetcher] 时间模式: API过滤后 {} 条, source={:?}",
        messages.len(), source
    );

    Ok(FetchResult {
        gap: detect_gap(&messages),
        messages,
        source,
    })
}

/// 将 pool 中的 API 原始 JSON 批量写入 pool（back-seeding）
async fn back_seed_pool(pool: &Pool, raw: &[Value], group_id: i64, cutoff: i64) {
    for value in raw {
        let ts = value.get("time").and_then(Value::as_i64).unwrap_or(0);
        if ts < cutoff {
            continue;
        }
        if let Some(msg) = PoolMessage::from_api_value(value, group_id) {
            pool.push(msg).await;
        }
    }
}

async fn fetch_api_until_cutoff(
    api: &ApiClient,
    group_id: i64,
    cutoff: i64,
) -> Result<(Vec<Value>, bool, Option<i64>)> {
    let mut all = Vec::<Value>::new();
    let mut seen_ids = std::collections::HashSet::<i64>::new();
    let mut page_seq: Option<i64> = None;
    let mut reached_cutoff = false;
    let mut earliest_ts: Option<i64> = None;

    for _ in 0..50 {
        let page = api
            .get_group_msg_history_paged(group_id, 5000, page_seq)
            .await
            .context("分页拉取群消息历史失败")?;

        if page.is_empty() {
            break;
        }

        let page_earliest = page.first().and_then(|m| m.get("time")).and_then(Value::as_i64);
        if let Some(ts) = page_earliest {
            earliest_ts = Some(earliest_ts.map_or(ts, |old| old.min(ts)));
            if ts <= cutoff {
                reached_cutoff = true;
            }
        }

        for msg in &page {
            let msg_id = msg.get("message_id").and_then(Value::as_i64).unwrap_or(0);
            if msg_id != 0 {
                if seen_ids.insert(msg_id) {
                    all.push(msg.clone());
                }
            } else {
                all.push(msg.clone());
            }
        }

        if reached_cutoff || page.len() < 5000 {
            break;
        }

        let next_seq = page
            .first()
            .and_then(|m| m.get("message_seq").and_then(Value::as_i64))
            .or_else(|| page.first().and_then(|m| m.get("message_id").and_then(Value::as_i64)));

        if next_seq.is_none() || next_seq == page_seq {
            break;
        }
        page_seq = next_seq;
    }

    Ok((all, reached_cutoff, earliest_ts))
}
