use once_cell::sync::OnceCell;
use serde::Deserialize;

use crate::kernel::error::AppError;

/// 全局配置单例
static CONFIG: OnceCell<Config> = OnceCell::new();

// ── 顶层结构 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub napcat: NapcatConfig,
    pub bot: BotConfig,
    #[serde(default)]
    pub pool: PoolConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// 监听地址，默认 "0.0.0.0"
    #[serde(default = "default_host")]
    pub host: String,
    /// 监听端口，默认 8080
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct NapcatConfig {
    /// NapCat/go-cqhttp HTTP API 地址，例如 "http://127.0.0.1:3000"
    pub url: String,
    /// Bearer Token（可选）
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BotConfig {
    /// 允许响应的群号白名单
    pub whitelist: Vec<i64>,
    /// 用户白名单：非空时仅响应列表内的 QQ 号
    /// 优先级高于 user_blacklist，两者同时设置时黑名单无效
    #[serde(default)]
    pub user_whitelist: Vec<i64>,
    /// 用户黑名单：永远忽略这些 QQ 号（user_whitelist 非空时此项无效）
    #[serde(default)]
    pub user_blacklist: Vec<i64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    /// OpenAI 兼容 API 地址
    #[serde(default = "default_llm_url")]
    pub api_url: String,
    /// API Key
    pub api_key: String,
    /// 模型名称
    #[serde(default = "default_llm_model")]
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct PoolConfig {
    /// 每个群的内存缓冲最大消息条数，默认 3000
    #[serde(default = "default_pool_capacity")]
    pub per_group_capacity: usize,
    /// 内存淘汰阈值（秒），超过此时间的消息被清理，默认 1d
    #[serde(default = "default_pool_evict")]
    pub evict_after_secs: i64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            per_group_capacity:      default_pool_capacity(),
            evict_after_secs:        default_pool_evict(),
        }
    }
}

/// 日志配置
#[derive(Debug, Deserialize)]
pub struct LogConfig {
    /// 日志文件目录 — 仅编译时启用 core-log-file 后生效
    #[cfg(feature = "core-log-file")]
    pub log_dir: Option<String>,
    /// 保留天数（启动时清理超期日志文件），默认 30 — 仅 core-log-file
    #[cfg(feature = "core-log-file")]
    #[serde(default = "default_log_max_days")]
    pub max_days: u32,
    /// 日志级别（trace/debug/info/warn/error），默认 info
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "core-log-file")]
            log_dir:  None,
            #[cfg(feature = "core-log-file")]
            max_days: default_log_max_days(),
            level:    default_log_level(),
        }
    }
}

// ── 默认值 ─────────────────────────────────────────────────────────────────────
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 8080 }
fn default_llm_url() -> String { "https://api.deepseek.com/v1".to_string() }
fn default_llm_model() -> String { "deepseek-chat".to_string() }
fn default_pool_capacity() -> usize { 3000 }
fn default_pool_evict() -> i64 { 86400 }
fn default_log_max_days() -> u32 { 30 }
fn default_log_level() -> String { "info".to_string() }

// ── 加载逻辑 ───────────────────────────────────────────────────────────────────

impl Config {
    /// 从 `config.toml` + `.env` 加载配置
    /// .env 中的变量优先级更高，可覆盖 TOML 中的值
    pub fn load() -> Result<Self, AppError> {
        // 加载 .env（文件不存在时静默跳过）
        dotenvy::dotenv().ok();

        let toml_str = std::fs::read_to_string("config.toml")
            .map_err(|e| AppError::Config(format!("无法读取 config.toml: {e}")))?;

        let mut config: Config = toml::from_str(&toml_str)?;

        // 空字符串 token 视为未配置
        if config.napcat.token.as_deref() == Some("") {
            config.napcat.token = None;
        }

        // .env 环境变量覆盖
        if let Ok(v) = std::env::var("NAPCAT_URL") {
            config.napcat.url = v;
        }
        if let Ok(v) = std::env::var("NAPCAT_TOKEN") {
            config.napcat.token = if v.is_empty() { None } else { Some(v) };
        }
        if let Ok(v) = std::env::var("SERVER_HOST") {
            config.server.host = v;
        }
        if let Ok(v) = std::env::var("SERVER_PORT") {
            config.server.port = v
                .parse()
                .map_err(|_| AppError::Config("SERVER_PORT 必须是有效端口号".into()))?;
        }

        Ok(config)
    }

    /// 获取全局配置（初始化后调用）
    pub fn global() -> &'static Config {
        CONFIG.get().expect("Config 未初始化，请先调用 config::init()")
    }
}

/// 初始化全局配置（在 main 中调用一次）
pub fn init() -> Result<(), AppError> {
    let config = Config::load()?;
    CONFIG
        .set(config)
        .map_err(|_| AppError::Config("Config 已被初始化".into()))
}
