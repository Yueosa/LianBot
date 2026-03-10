use std::collections::HashMap;

use once_cell::sync::OnceCell;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::kernel::error::AppError;

/// 全局配置单例
static CONFIG: OnceCell<KernelConfig> = OnceCell::new();

// ══════════════════════════════════════════════════════════════════════════════
//  KernelConfig — kernel 层强类型配置（config.toml）
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct KernelConfig {
    /// 监听地址，默认 "0.0.0.0"
    #[serde(default = "default_host")]
    pub host: String,
    /// 监听端口，默认 8080
    #[serde(default = "default_port")]
    pub port: u16,
}

// ── 默认值 ─────────────────────────────────────────────────────────────────────
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 8080 }

// ── 加载逻辑 ───────────────────────────────────────────────────────────────────

impl KernelConfig {
    /// 从 `config.toml` + `.env` 加载配置
    pub fn load() -> Result<Self, AppError> {
        dotenvy::dotenv().ok();

        let toml_str = std::fs::read_to_string("config.toml")
            .map_err(|e| AppError::Config(format!("无法读取 config.toml: {e}")))?;

        let mut config: KernelConfig = toml::from_str(&toml_str)?;

        // .env 环境变量覆盖 kernel 层字段
        if let Ok(v) = std::env::var("SERVER_HOST") {
            config.host = v;
        }
        if let Ok(v) = std::env::var("SERVER_PORT") {
            config.port = v
                .parse()
                .map_err(|_| AppError::Config("SERVER_PORT 必须是有效端口号".into()))?;
        }

        Ok(config)
    }

    pub fn global() -> &'static KernelConfig {
        CONFIG.get().expect("KernelConfig 未初始化，请先调用 config::init()")
    }
}

/// 初始化全局配置（在 main 中调用一次）
pub fn init() -> Result<(), AppError> {
    let config = KernelConfig::load()?;
    CONFIG
        .set(config)
        .map_err(|_| AppError::Config("KernelConfig 已被初始化".into()))
}

// ══════════════════════════════════════════════════════════════════════════════
//  LayerConfig — 通用层级配置容器（runtime / logic 层复用）
// ══════════════════════════════════════════════════════════════════════════════

/// 从 TOML 文件加载为 `HashMap<String, Value>`，供各层按需提取 section。
/// 文件不存在时静默返回空 HashMap，各段缺失时返回 `T::default()`。
pub struct LayerConfig {
    raw: HashMap<String, toml::Value>,
}

impl LayerConfig {
    /// 加载指定 TOML 文件。文件不存在则返回空容器（不报错）。
    pub fn load(path: &str) -> Result<Self, AppError> {
        let raw = if std::path::Path::new(path).exists() {
            let s = std::fs::read_to_string(path)
                .map_err(|e| AppError::Config(format!("无法读取 {path}: {e}")))?;
            toml::from_str::<HashMap<String, toml::Value>>(&s)?
        } else {
            HashMap::new()
        };
        Ok(Self { raw })
    }

    /// 读取指定 section，反序列化为 `T`。
    /// section 缺失或格式错误时返回 `T::default()`。
    pub fn section<T: DeserializeOwned + Default>(&self, key: &str) -> T {
        self.raw
            .get(key)
            .and_then(|v| v.clone().try_into::<T>().ok())
            .unwrap_or_default()
    }

    /// 读取指定 section，section 缺失时返回 None。
    pub fn section_opt<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.raw
            .get(key)
            .and_then(|v| v.clone().try_into::<T>().ok())
    }

    /// 用环境变量覆盖指定 section 下的某个字段。
    /// 若环境变量存在，则写入 `raw[section][field] = Value::String(val)`。
    /// section 不存在时自动创建。
    pub fn env_override(&mut self, section: &str, field: &str, env_var: &str) {
        if let Ok(val) = std::env::var(env_var) {
            let table = self.raw
                .entry(section.to_string())
                .or_insert_with(|| toml::Value::Table(Default::default()));
            if let toml::Value::Table(t) = table {
                t.insert(field.to_string(), toml::Value::String(val));
            }
        }
    }
}
