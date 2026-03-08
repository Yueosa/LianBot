use anyhow::Context;

use crate::runtime::api::ApiClient;

use super::model::PoolMessage;
use super::traits::MessagePool;
use super::Pool;

/// 拉取白名单群的历史消息填充消息池（冷启动 back-seeding）。
/// 由 boot.rs 在启动时 `tokio::spawn` 调用。
pub async fn seed_from_history(api: &ApiClient, pool: &Pool, groups: Vec<i64>) {
    if groups.is_empty() {
        tracing::info!("[pool-seed] 无已开启的群，跳过启动预热");
        return;
    }

    tracing::info!("[pool-seed] 启动预热开始：{} 个群", groups.len());
    let mut total = 0usize;

    for gid in groups {
        match seed_one_group(api, pool, gid).await {
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

async fn seed_one_group(api: &ApiClient, pool: &Pool, group_id: i64) -> anyhow::Result<usize> {
    let raw = api
        .get_group_msg_history_paged(group_id, 3000, None)
        .await
        .with_context(|| format!("拉取群 {} 历史消息失败", group_id))?;

    let mut seeded = 0usize;
    for value in raw {
        if let Some(msg) = PoolMessage::from_api_value(&value, group_id) {
            pool.push(msg).await;
            seeded += 1;
        }
    }

    Ok(seeded)
}
