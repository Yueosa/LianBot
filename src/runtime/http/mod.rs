// ── runtime::http ──────────────────────────────────────────────────────────────
//
// 统一 HTTP 客户端管理模块。
//
// 职责：
//   - 提供全局共享的 reqwest::Client
//   - 统一配置超时、连接池、TCP keepalive 等参数
//   - 避免各模块重复创建 HTTP 客户端，提高连接池复用效率
//
// 使用方：
//   - runtime::api（NapCat API 客户端）
//   - runtime::llm（LLM API 客户端）
//   - commands 层（acg、alive、world、dress 等外部 API 命令）
//   - runtime::image（图片下载，未来实现）

use std::sync::OnceLock;
use std::time::Duration;
use serde::Deserialize;

// ── 配置 ──────────────────────────────────────────────────────────────────────

/// runtime.toml `[http]` 段。
#[derive(Debug, Deserialize, Clone)]
pub struct HttpConfig {
    /// 请求保底超时（秒），默认 30s
    /// 各模块可在请求时覆盖此超时（如 LLM 使用 120s，commands 使用 10s）
    #[serde(default = "HttpConfig::default_timeout")]
    pub timeout_secs: u64,

    /// 连接池空闲超时（秒），默认 30s
    #[serde(default = "HttpConfig::default_pool_idle_timeout")]
    pub pool_idle_timeout_secs: u64,

    /// 每个 host 的最大空闲连接数，默认 10
    #[serde(default = "HttpConfig::default_pool_max_idle")]
    pub pool_max_idle_per_host: usize,

    /// TCP keepalive 间隔（秒），默认 60s
    #[serde(default = "HttpConfig::default_tcp_keepalive")]
    pub tcp_keepalive_secs: u64,
}

impl HttpConfig {
    fn default_timeout() -> u64 { 30 }
    fn default_pool_idle_timeout() -> u64 { 30 }
    fn default_pool_max_idle() -> usize { 10 }
    fn default_tcp_keepalive() -> u64 { 60 }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            timeout_secs: Self::default_timeout(),
            pool_idle_timeout_secs: Self::default_pool_idle_timeout(),
            pool_max_idle_per_host: Self::default_pool_max_idle(),
            tcp_keepalive_secs: Self::default_tcp_keepalive(),
        }
    }
}

// ── 全局客户端 ────────────────────────────────────────────────────────────────

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 初始化全局 HTTP 客户端（在 runtime::init() 中调用一次）。
///
/// 配置说明：
/// - timeout：保底超时，各模块可在请求时通过 `.timeout()` 覆盖
/// - pool_idle_timeout：连接池空闲超时，超时后连接被关闭
/// - pool_max_idle_per_host：每个 host 的最大空闲连接数
/// - tcp_keepalive：TCP 层 keepalive 探测间隔
/// - redirect：自动跟随最多 10 次重定向
pub fn init(config: HttpConfig) {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .pool_idle_timeout(Duration::from_secs(config.pool_idle_timeout_secs))
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .tcp_keepalive(Duration::from_secs(config.tcp_keepalive_secs))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("HTTP 客户端初始化失败")
    });
}

/// 获取全局 HTTP 客户端。
///
/// # Panics
/// 如果在调用 `init()` 之前调用此函数，会 panic。
/// 正常情况下，`init()` 在 `runtime::init()` 中被调用，早于所有使用方。
pub fn client() -> &'static reqwest::Client {
    HTTP_CLIENT.get().expect("HTTP 客户端未初始化，请确保在 runtime::init() 中调用了 http::init()")
}
