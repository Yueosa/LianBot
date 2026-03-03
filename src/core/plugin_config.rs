use std::collections::HashMap;

use once_cell::sync::OnceCell;
use serde::de::DeserializeOwned;

use crate::core::error::AppError;

static PLUGIN_CONFIG: OnceCell<PluginConfig> = OnceCell::new();

/// 插件私有配置，从 `plugins.toml` 加载。
/// 每个插件/命令通过 `get_section::<T>("name")` 获取自己的配置段，
/// 文件不存在或段缺失时返回 `T::default()`。
pub struct PluginConfig {
    raw: HashMap<String, toml::Value>,
}

impl PluginConfig {
    fn load() -> Result<Self, AppError> {
        let raw = if std::path::Path::new("plugins.toml").exists() {
            let s = std::fs::read_to_string("plugins.toml")
                .map_err(|e| AppError::Config(format!("无法读取 plugins.toml: {e}")))?;
            toml::from_str::<HashMap<String, toml::Value>>(&s)?
        } else {
            HashMap::new() // 文件不存在 → 全部使用默认值
        };
        Ok(Self { raw })
    }

    /// 读取指定 section，反序列化为 `T`。
    /// section 缺失或格式错误时返回 `T::default()`。
    pub fn get_section<T: DeserializeOwned + Default>(&self, key: &str) -> T {
        self.raw
            .get(key)
            .and_then(|v| toml::to_string(v).ok())      // Value → TOML 字符串
            .and_then(|s| toml::from_str::<T>(&s).ok()) // 字符串 → T
            .unwrap_or_default()
    }

    pub fn global() -> &'static PluginConfig {
        PLUGIN_CONFIG.get().expect("PluginConfig 未初始化，请先调用 plugin_config::init()")
    }
}

/// 初始化全局插件配置（在 main 中调用一次）。
/// `plugins.toml` 不存在时静默跳过（各插件使用编译期默认值）。
pub fn init() -> Result<(), AppError> {
    let config = PluginConfig::load()?;
    PLUGIN_CONFIG
        .set(config)
        .map_err(|_| AppError::Config("PluginConfig 已重复初始化".into()))
}
