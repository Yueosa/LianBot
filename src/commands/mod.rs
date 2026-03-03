#[cfg(feature = "cmd-ping")]  pub mod ping;
#[cfg(feature = "cmd-help")]  pub mod help;
#[cfg(feature = "cmd-img")]   pub mod img;
#[cfg(feature = "cmd-stalk")] pub mod stalk;
#[cfg(feature = "cmd-smy")]   pub mod smy;
#[cfg(feature = "cmd-alive")] pub mod alive;
#[cfg(feature = "cmd-world")] pub mod world;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::core::{
    api::ApiClient,
    config::Config,
    parser::ParamValue,
    pool::Pool,
    registry::CommandRegistry,
    ws::WsManager,
};

// ── Command 元数据类型 ────────────────────────────────────────────────────────

/// 命令类型：决定 dispatcher 如何路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// `/ping`、`/alive` — 由 `/` 开头触发，不接受参数
    Simple,
    /// `<smy>`、`<img>` — 由 `<>` 包裹触发，接受参数
    Advanced,
}

/// 参数值约束，用于 dispatcher 自动校验和 `--help` 生成类型提示。
#[derive(Debug, Clone, Copy)]
pub enum ValueConstraint {
    /// 任意字符串，不校验
    Any,
    /// 整数范围，`min`/`max` 为 `None` 表示无限制
    Integer { min: Option<i64>, max: Option<i64> },
    /// 枚举值，输入必须是其中之一
    OneOf(&'static [&'static str]),
}

/// 参数值类型。
#[derive(Debug, Clone, Copy)]
pub enum ParamKind {
    /// 纯 flag，`--ai`，无值
    Flag,
    /// 携带值，并附带约束条件
    Value(ValueConstraint),
}

/// 单条参数规格说明，供 dispatcher 校验和 `--help` 自动生成使用。
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// 所有键别名，如 `&["-n", "--count"]`
    pub keys: &'static [&'static str],
    pub kind: ParamKind,
    pub required: bool,
    pub help: &'static str,
}

/// 可选依赖声明，供 Phase 4 feature gate 检查使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    /// 需要 `plugins.toml` 中的配置段（如 LLM）
    Config,
    /// 需要 `WsManager`（WebSocket 客户端已连接）
    Ws,
    /// 需要消息池（Phase 3）
    Pool,
}

// ── Command Trait ─────────────────────────────────────────────────────────────

/// 所有命令实现此 trait。
#[async_trait]
pub trait Command: Send + Sync {
    /// 命令主名，如 `"img"`、`"/ping"`
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

    /// 可选依赖声明（默认为空）
    fn dependencies(&self) -> &[Dependency] {
        &[]
    }

    /// 执行命令
    async fn execute(&self, ctx: CommandContext) -> anyhow::Result<()>;
}

// ── 命令上下文 ─────────────────────────────────────────────────────────────────

pub struct CommandContext {
    /// 触发命令的群号
    pub group_id: i64,
    /// 发送者 QQ 号
    pub user_id: i64,
    /// 解析后的参数 map
    pub params: HashMap<String, ParamValue>,
    /// OneBot API 客户端（Arc 共享）
    pub api: Arc<ApiClient>,
    /// WebSocket 连接管理器（Arc 共享）
    pub ws: Arc<WsManager>,
    /// 全局配置
    pub config: &'static Config,
    /// 命令注册表（供 /help 等命令枚举全部命令）
    pub registry: Arc<CommandRegistry>,
    /// 消息池（per-group 内存缓冲）
    pub pool: Arc<Pool>,
}

impl CommandContext {
    // ── 参数查询 ──────────────────────────────────────────────────────────────

    /// 按多个别名查找参数值字符串。
    /// 例如：`ctx.get(&["-u", "--url"])` 会按顺序尝试每个 key。
    pub fn get(&self, keys: &[&str]) -> Option<&str> {
        for &key in keys {
            if let Some(v) = self.params.get(key) {
                if let Some(s) = v.as_str() {
                    return Some(s);
                }
            }
        }
        None
    }
}
