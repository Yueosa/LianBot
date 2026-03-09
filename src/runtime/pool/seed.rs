use anyhow::Context;
use serde_json::Value;

use crate::runtime::api::ApiClient;

use super::model::{PoolConfig, PoolMessage};
use super::traits::MessagePool;
use super::Pool;

/// 拉取白名单群的历史消息填充消息池（冷启动 back-seeding）。
/// 分页回溯直到覆盖 evict_after_secs 窗口，确保 smy 首次请求即可命中 pool。
/// 由 boot.rs 在启动时 `tokio::spawn` 调用。
pub async fn seed_from_history(api: &ApiClient, pool: &Pool, groups: Vec<i64>) {
    if groups.is_empty() {
        tracing::info!("[pool-seed] 无已开启的群，跳过启动预热");
        return;
    }

    let cfg: PoolConfig = crate::runtime::config::section("pool");
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        .saturating_sub(cfg.evict_after_secs);

    tracing::info!("[pool-seed] 启动预热开始：{} 个群, cutoff={cutoff}", groups.len());
    let mut total = 0usize;

    for gid in groups {
        match seed_one_group(api, pool, gid, cutoff).await {
            Ok(n) => {
                total += n;
                tracing::info!("[pool-seed] 群 {gid} 预热完成：{n} 条");
            }
            Err(e) => {
                tracing::warn!("[pool-seed] 群 {gid} 预热失败: {e:#}");
            }
        }
    }

    tracing::info!("[pool-seed] 启动预热结束：累计 {total} 条");
}

/// 单群分页 seed：以 3000 条/页向前翻页，直到最早消息 ≤ cutoff 或服务端穷尽。
async fn seed_one_group(
    api: &ApiClient,
    pool: &Pool,
    group_id: i64,
    cutoff: i64,
) -> anyhow::Result<usize> {
    let mut seeded = 0usize;
    let mut page_seq: Option<i64> = None;

    for _ in 0..20 {
        let raw = api
            .get_group_msg_history_paged(group_id, 3000, page_seq)
            .await
            .with_context(|| format!("拉取群 {} 历史消息失败", group_id))?;

        if raw.is_empty() {
            break;
        }

        let mut reached_cutoff = false;
        let mut earliest_ts = i64::MAX;

        for value in &raw {
            let ts = value.get("time").and_then(Value::as_i64).unwrap_or(0);
            if ts < earliest_ts {
                earliest_ts = ts;
            }
            if ts < cutoff {
                reached_cutoff = true;
            }
            if let Some(msg) = PoolMessage::from_api_value(value, group_id) {
                pool.push(msg).await;
                seeded += 1;
            }
        }

        if reached_cutoff || raw.len() < 3000 {
            break;
        }

        // 取最早消息的 message_seq 作为下一页起点
        let next_seq = raw
            .first()
            .and_then(|m| m.get("message_seq").and_then(Value::as_i64))
            .or_else(|| raw.first().and_then(|m| m.get("message_id").and_then(Value::as_i64)));

        if next_seq.is_none() || next_seq == page_seq {
            break;
        }
        page_seq = next_seq;
    }

    Ok(seeded)
}
