use async_trait::async_trait;

use super::context::CommandContext;
use super::params::ParamSpec;

// ── Command 元数据类型 ────────────────────────────────────────────────────────

/// 命令类型：决定 dispatcher 如何路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// `!!ping`、`!!alive` — 由可配置前缀（`cmd_prefix`）开头触发
    Simple,
    /// `<smy>` — 由 `<>` 包裹触发，接受参数
    Advanced,
}

/// 运行时依赖声明，Dispatcher 在执行命令前自动检查可用性。
/// 不可用时返回友好提示而非 runtime error。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    /// 需要 `logic.toml` 中的配置段（如 LLM）
    Config,
    /// 需要 `WsManager`（WebSocket 客户端已连接）
    Ws,
    /// 需要消息池
    Pool,
}

// ── Command Trait ─────────────────────────────────────────────────────────────

/// 所有命令实现此 trait。
#[async_trait]
pub trait Command: Send + Sync {
    /// 命令主名（纯名字，不含前缀），如 `"ping"`、`"smy"`
    fn name(&self) -> &str;

    /// 别名列表（默认为空），返回静态切片
    fn aliases(&self) -> &[&str] {
        &[]
    }

    /// 单行或多行帮助描述
    fn help(&self) -> &str;

    /// 命令类型（必须实现）
    fn kind(&self) -> CommandKind;

    /// 声明的参数规格，用于校验和帮助生成（默认为空）
    fn declared_params(&self) -> &[ParamSpec] {
        &[]
    }

    /// 运行时依赖声明，Dispatcher 在执行前自动检查可用性。
    fn dependencies(&self) -> &[Dependency] {
        &[]
    }

    /// 执行此命令所需的最低角色，默认 `Member`（所有人可用）。
    /// 管理命令覆盖为 `Role::Owner`。
    fn required_role(&self) -> crate::runtime::permission::Role {
        crate::runtime::permission::Role::Member
    }

    /// Simple 命令是否接受尾部参数（默认 false）。
    /// 为 true 时 dispatcher 将 trailing 合并为 `_args` 传入 `ctx.params`。
    fn accepts_trailing(&self) -> bool {
        false
    }

    /// 返回该命令作为 LLM tool 时的自然语言描述。
    /// 返回 `None` 表示不暴露给 LLM（默认）。
    fn tool_description(&self) -> Option<&str> {
        None
    }

    /// 执行命令
    async fn execute(&self, ctx: CommandContext) -> anyhow::Result<()>;
}
