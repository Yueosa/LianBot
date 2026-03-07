#[cfg(feature = "cmd-ping")]  pub mod ping;
#[cfg(feature = "cmd-help")]  pub mod help;
#[cfg(feature = "cmd-acg")]   pub mod acg;
#[cfg(feature = "cmd-stalk")] pub mod stalk;
#[cfg(feature = "cmd-smy")]   pub mod smy;
#[cfg(feature = "cmd-alive")] pub mod alive;
#[cfg(feature = "cmd-world")] pub mod world;
pub mod admin;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::runtime::permission::{AccessControl, BotUser};
use crate::runtime::{
    api::ApiClient,
    parser::ParamValue,
    pool::Pool,
    registry::CommandRegistry,
    typ::MessageSegment,
    ws::WsManager,
};

// ── 共享 HTTP 客户端 ──────────────────────────────────────────────────────────

/// 命令层共享的 reqwest::Client（OnceLock 惰性初始化，进程内唯一）。
/// 配置：跟随最多 10 次重定向、10 秒超时。
/// acg / alive / world 等外部 API 命令统一使用，避免每次调用新建 Client。
pub fn http_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest::Client 初始化失败")
    })
}

// ── Command 元数据类型 ────────────────────────────────────────────────────────

/// 命令类型：决定 dispatcher 如何路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// `!!ping`、`!!alive` — 由可配置前缀（`cmd_prefix`）开头触发
    Simple,
    /// `<smy>` — 由 `<>` 包裹触发，接受参数
    Advanced,
}

/// 参数值约束，用于 dispatcher 自动校验和 `--help` 生成类型提示。
#[derive(Debug, Clone, Copy)]
pub enum ValueConstraint {
    /// 任意字符串，不校验
    Any,
    /// 整数范围，`min`/`max` 为 `None` 表示无限制
    Integer { min: Option<i64>, max: Option<i64> },
    /// 枚举值，输入必须是其中之一（当前暂无命令使用，保留供未来扩展）
    #[allow(dead_code)]
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
    /// 所有键别名，如 `&["-t", "--time"]`
    pub keys: &'static [&'static str],
    pub kind: ParamKind,
    pub required: bool,
    pub help: &'static str,
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

    /// 执行命令
    async fn execute(&self, ctx: CommandContext) -> anyhow::Result<()>;
}

// ── 命令上下文 ─────────────────────────────────────────────────────────────────

pub struct CommandContext {
    /// 触发命令的群号
    pub group_id: i64,
    /// 触发消息的 message_id（用于回复等操作，部分事件可能无此字段）
    pub message_id: Option<i64>,
    /// 发送者的虚拟用户对象（包含 user_id、role、status）
    pub bot_user: BotUser,
    /// 原始消息段列表（含图片/at/回复等非文本 segment）
    pub segments: Vec<MessageSegment>,
    /// 解析后的参数 map
    pub params: HashMap<String, ParamValue>,
    /// OneBot API 客户端（Arc 共享）
    pub api: Arc<ApiClient>,
    /// WebSocket 连接管理器（Arc 共享）
    pub ws: Arc<WsManager>,
    /// 命令前缀（从 runtime 配置提取）
    pub cmd_prefix: String,
    /// 命令注册表（供 /help 等命令枚举全部命令）
    pub registry: Arc<CommandRegistry>,
    /// 消息池（per-group 内存缓冲，可选）
    pub pool: Option<Arc<Pool>>,
    /// 准入控制（block/unblock、enable/disable 等管理操作）
    pub access: Arc<AccessControl>,
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

// ── 命令自注册 ────────────────────────────────────────────────────────────────

/// 向 App 构建器注册所有已启用 feature 的命令。
pub fn register(app: &mut crate::kernel::app::App) {
    app.command(Arc::new(admin::AdminCommand));
    #[cfg(feature = "cmd-ping")]  app.command(Arc::new(ping::PingCommand));
    #[cfg(feature = "cmd-help")]  app.command(Arc::new(help::HelpCommand));
    #[cfg(feature = "cmd-acg")]   app.command(Arc::new(acg::AcgCommand));
    #[cfg(feature = "cmd-stalk")] app.command(Arc::new(stalk::StalkCommand));
    #[cfg(feature = "cmd-smy")]   app.command(Arc::new(smy::SmyCommand));
    #[cfg(feature = "cmd-alive")] app.command(Arc::new(alive::AliveCommand));
    #[cfg(feature = "cmd-world")] app.command(Arc::new(world::WorldCommand));
}
