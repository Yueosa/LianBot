use thiserror::Error;

/// Kernel 启动链路专用错误（config 加载 / 文件 IO / TOML 解析）。
/// 其他模块使用 `anyhow` 或自定义 error，不依赖此类型。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("配置加载失败: {0}")]
    Config(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML 解析失败: {0}")]
    Toml(#[from] toml::de::Error),
}
