#[cfg(feature = "cmd-ping")]  pub mod ping;
#[cfg(feature = "cmd-help")]  pub mod help;
#[cfg(feature = "cmd-acg")]   pub mod acg;
#[cfg(feature = "cmd-stalk")] pub mod stalk;
#[cfg(feature = "cmd-smy")]   pub mod smy;
#[cfg(feature = "cmd-alive")] pub mod alive;
#[cfg(feature = "cmd-world")] pub mod world;
#[cfg(feature = "cmd-dress")] pub mod dress;
#[cfg(feature = "cmd-sign")] pub mod sign;
#[cfg(feature = "cmd-send")] pub mod send;
pub mod admin;

mod core;
pub use self::core::*;

use std::sync::Arc;

// ── 命令自注册 ────────────────────────────────────────────────────────────────

/// 命令注册摘要
#[derive(Default)]
pub struct CommandsSummary {
    /// 已注册的命令数量
    pub count: usize,
    /// 命令名称列表
    pub names: Vec<String>,
}

/// 向 App 构建器注册所有已启用 feature 的命令。
pub fn register(app: &mut crate::kernel::app::App) -> CommandsSummary {
    let mut summary = CommandsSummary::default();

    app.command(Arc::new(admin::AdminCommand));
    summary.names.push("admin".to_string());

    #[cfg(feature = "cmd-ping")]
    {
        app.command(Arc::new(ping::PingCommand));
        summary.names.push("ping".to_string());
    }

    #[cfg(feature = "cmd-help")]
    {
        app.command(Arc::new(help::HelpCommand));
        summary.names.push("help".to_string());
    }

    #[cfg(feature = "cmd-acg")]
    {
        app.command(Arc::new(acg::AcgCommand));
        summary.names.push("acg".to_string());
    }

    #[cfg(feature = "cmd-stalk")]
    {
        app.command(Arc::new(stalk::StalkCommand));
        summary.names.push("stalk".to_string());
    }

    #[cfg(feature = "cmd-smy")]
    {
        app.command(Arc::new(smy::SmyCommand));
        summary.names.push("smy".to_string());
    }

    #[cfg(feature = "cmd-alive")]
    {
        app.command(Arc::new(alive::AliveCommand));
        summary.names.push("alive".to_string());
    }

    #[cfg(feature = "cmd-world")]
    {
        app.command(Arc::new(world::WorldCommand));
        summary.names.push("world".to_string());
    }

    #[cfg(feature = "cmd-dress")]
    {
        app.command(Arc::new(dress::DressCommand));
        summary.names.push("dress".to_string());
    }

    #[cfg(feature = "cmd-sign")]
    {
        app.command(Arc::new(sign::SignCommand));
        summary.names.push("sign".to_string());
    }

    #[cfg(feature = "cmd-send")]
    {
        app.command(Arc::new(send::SendCommand));
        summary.names.push("send".to_string());
    }

    summary.count = summary.names.len();
    summary
}
