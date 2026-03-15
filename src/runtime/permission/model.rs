/// 消息发生在哪里：群聊或私聊。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// 群聊，携带 group_id
    Group(i64),
    /// 私聊，携带对方 QQ 号
    Private(i64),
}

/// Bot 内虚拟角色（全局，不区分群）。
///
/// 排序：`Member < Owner`，用于命令权限检查。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// 其他所有用户
    Member,
    /// Bot 主人，来自 config.toml，不落库，权限最高
    Owner,
}

/// 一次消息交互中代表一个真实 QQ 用户的虚拟用户对象。
///
/// 由 `Dispatcher::resolve_user()` 在入口一次性构造，
/// 之后传给所有 handler（Command / Session / Service），不再重复查权限。
/// 黑名单检查在 dispatcher 网关层完成，通过网关的用户一定是 Normal 状态。
#[derive(Debug, Clone)]
pub struct BotUser {
    /// QQ 号，全局唯一真实标识
    #[allow(dead_code)]
    pub user_id: i64,
    /// 消息上下文（群聊 or 私聊）
    pub scope: Scope,
    /// Bot 内虚拟角色
    pub role: Role,
}

impl BotUser {
    /// 是否是 Bot 主人。
    #[allow(dead_code)]
    pub fn is_owner(&self) -> bool {
        self.role == Role::Owner
    }
}
