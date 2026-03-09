use async_trait::async_trait;

use crate::runtime::permission::Scope;
use super::model::{MsgStatus, PoolMessage, ProcessRecord};

/// 消息池统一接口。当前实现为 InMemory（后续可扩展为 Actor/其他存储后端）。
#[async_trait]
pub trait MessagePool: Send + Sync {
    /// 写入一条消息（容量/时间淘汰由实现层自动处理）
    async fn push(&self, msg: PoolMessage);

    /// 标记消息处理完成（dispatcher 在 cmd.execute() 之后调用）。
    /// 在对应 scope 的队列中反向查找 msg_id 并更新状态。
    async fn mark(&self, msg_id: i64, scope: &Scope, status: MsgStatus, record: ProcessRecord);

    /// internal-only：读取指定 scope 最近 n 条消息（时序: 旧 → 新）。
    /// 不保证时间连续性，不作为命令层对外语义使用。
    #[doc(hidden)]
    async fn recent_internal(&self, scope: &Scope, n: usize) -> Vec<PoolMessage>;

    /// 读取指定 scope 在 [since, until] 时间范围内的消息（秒级时间戳）
    async fn range(&self, scope: &Scope, since: i64, until: i64) -> Vec<PoolMessage>;

    /// 返回指定 scope 在 pool 中最早一条消息的时间戳（秒级）。
    /// 用于判断 pool 是否覆盖了某个时间窗口：若 oldest <= cutoff，则覆盖完整。
    /// 无任何消息时返回 None。
    async fn oldest_timestamp(&self, scope: &Scope) -> Option<i64>;
}
