//! Runtime 层配置，从 `runtime.toml` 加载。
//!
//! 包含 napcat、parser、pool、log 等运行时基础设施配置。
//! 各模块通过 `section::<T>(key)` 按需提取自己的配置段。

use once_cell::sync::OnceCell;
use serde::de::DeserializeOwned;

use crate::kernel::config::LayerConfig;
use crate::kernel::error::AppError;

static RT_CONFIG: OnceCell<LayerConfig> = OnceCell::new();

/// 初始化 runtime 层配置（在 boot 中调用一次）。
/// `runtime.toml` 不存在时静默跳过（各段使用编译期默认值）。
pub fn init() -> Result<(), AppError> {
    let mut layer = LayerConfig::load("runtime.toml")?;

    // .env 環境變量覆蓋 napcat 段
    layer.env_override("napcat", "url", "NAPCAT_URL");
    layer.env_override("napcat", "token", "NAPCAT_TOKEN");

    RT_CONFIG
        .set(layer)
        .map_err(|_| AppError::Config("RuntimeConfig 已重复初始化".into()))
}

/// 获取 runtime 层配置全局单例。
pub fn global() -> &'static LayerConfig {
    RT_CONFIG.get().expect("RuntimeConfig 未初始化，请先调用 runtime::config::init()")
}

/// 便捷函数：提取 runtime 层指定 section。
pub fn section<T: DeserializeOwned + Default>(key: &str) -> T {
    global().section(key)
}
