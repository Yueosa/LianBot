use once_cell::sync::OnceCell;
use serde::Deserialize;

use crate::error::AppError;

/// 全局配置单例
static CONFIG: OnceCell<Config> = OnceCell::new();

// ── 顶层结构 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub napcat: NapcatConfig,
    pub bot: BotConfig,
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
}

// ── 默认值 ─────────────────────────────────────────────────────────────────────
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 8080 }

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
