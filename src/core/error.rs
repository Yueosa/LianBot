use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("配置加载失败: {0}")]
    Config(String),

    #[error("命令解析失败: {0}")]
    Parse(String),

    #[error("命令 {cmd} 参数错误: {detail}")]
    InvalidParam { cmd: String, detail: String },

    #[error("API 请求失败: {0}")]
    Api(#[from] reqwest::Error),

    #[error("JSON 序列化失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML 解析失败: {0}")]
    Toml(#[from] toml::de::Error),
}

/// 应用级 Result 别名
pub type Result<T> = std::result::Result<T, AppError>;
