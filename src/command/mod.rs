pub mod context;
pub mod registry;
pub mod commands;

pub use context::CommandContext;
pub use registry::CommandRegistry;

use async_trait::async_trait;

// ── Command Trait ─────────────────────────────────────────────────────────────

/// 所有命令实现此 trait。
/// - `name()` 返回主命令名（注册 key）
/// - `help()` 返回帮助文本
/// - `execute()` 是异步执行入口
#[async_trait]
pub trait Command: Send + Sync {
    /// 命令主名，如 `"img"`、`"/ping"`
    fn name(&self) -> &str;

    /// 别名列表（默认为空）
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }

    /// 单行帮助描述
    fn help(&self) -> &str;

    /// 执行命令
    async fn execute(&self, ctx: CommandContext) -> anyhow::Result<()>;
}
