//! Logic 层配置，从 `logic.toml` 加载。
//!
//! 包含 smy、github、alive 等业务逻辑的配置段。
//! 各模块通过 `section::<T>(key)` 按需提取自己的配置段。

use once_cell::sync::OnceCell;
use serde::de::DeserializeOwned;

use crate::kernel::config::LayerConfig;
use crate::kernel::error::AppError;

static LOGIC_CONFIG: OnceCell<LayerConfig> = OnceCell::new();

/// 初始化 logic 层配置（在 boot 中调用一次）。
/// `logic.toml` 不存在时静默跳过（各段使用编译期默认值）。
pub fn init() -> Result<(), AppError> {
    let layer = LayerConfig::load("logic.toml")?;
    LOGIC_CONFIG
        .set(layer)
        .map_err(|_| AppError::Config("LogicConfig 已重复初始化".into()))
}

/// 获取 logic 层配置全局单例。
pub fn global() -> &'static LayerConfig {
    LOGIC_CONFIG.get().expect("LogicConfig 未初始化，请先调用 logic_config::init()")
}

/// 便捷函数：提取 logic 层指定 section。
pub fn section<T: DeserializeOwned + Default>(key: &str) -> T {
    global().section(key)
}
