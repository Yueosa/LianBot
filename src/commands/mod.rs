#[cfg(feature = "cmd-ping")]  pub mod ping;
#[cfg(feature = "cmd-help")]  pub mod help;
#[cfg(feature = "cmd-acg")]   pub mod acg;
#[cfg(feature = "cmd-stalk")] pub mod stalk;
#[cfg(feature = "cmd-smy")]   pub mod smy;
#[cfg(feature = "cmd-alive")] pub mod alive;
#[cfg(feature = "cmd-world")] pub mod world;
pub mod admin;

mod core;
pub use self::core::*;

use std::sync::Arc;

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
