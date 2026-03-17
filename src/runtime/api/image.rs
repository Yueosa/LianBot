// ── runtime::api::image ────────────────────────────────────────────────────────
//
// QQ/NapCat 图片处理模块。
//
// 职责：
//   - 调用 NapCat 的 get_image API 获取图片信息
//   - 下载 QQ 图片（优先使用本地缓存，否则下载 URL）
//   - 处理 QQ 图片的特殊逻辑（file_id 转换等）
//
// 使用场景：
//   - vision 命令：识别用户发送的 QQ 图片
//   - 其他需要处理 QQ 图片的场景
//
// 注意：
//   - 外部 API 图片（如 acg、dress）直接使用 runtime::http::client() 下载
//   - 本模块只处理 QQ/NapCat 图片

use anyhow::{Context, Result};
use serde::Deserialize;

use super::ApiClient;
use crate::runtime::typ::MessageSegment;

// ── 数据结构 ──────────────────────────────────────────────────────────────────

/// get_image API 响应
#[derive(Debug, Deserialize)]
pub struct GetImageResponse {
    /// 本地文件路径（如果 NapCat 有缓存）
    pub file: Option<String>,
    /// 图片 URL（可以直接下载）
    pub url: Option<String>,
    /// 文件大小（字节）
    #[allow(dead_code)]
    pub size: Option<u64>,
}

// ── API 方法 ──────────────────────────────────────────────────────────────────

impl ApiClient {
    /// 获取图片信息（调用 NapCat get_image API）
    ///
    /// # 参数
    /// - `file_id`: 消息段中的 file 字段（如 "B6059CA07B0EB9300204D7CFEF5A9065.jpg"）
    ///
    /// # 返回
    /// - `file`: 本地文件路径（如果 NapCat 有缓存）
    /// - `url`: 图片 URL（可以直接下载）
    /// - `size`: 文件大小
    ///
    /// # 示例
    /// ```rust
    /// let img_info = api.get_image("B6059CA07B0EB9300204D7CFEF5A9065.jpg").await?;
    /// if let Some(file_path) = img_info.file {
    ///     // 使用本地文件
    /// } else if let Some(url) = img_info.url {
    ///     // 下载 URL
    /// }
    /// ```
    pub async fn get_image(&self, file_id: &str) -> Result<GetImageResponse> {
        let payload = serde_json::json!({
            "file": file_id,
        });

        let resp = self.post("get_image", &payload).await?;

        let data = resp.get("data")
            .context("get_image 响应缺少 data 字段")?;

        Ok(serde_json::from_value(data.clone())?)
    }

    /// 从消息段下载图片
    ///
    /// 优先使用本地文件路径，如果没有则下载 URL。
    ///
    /// # 参数
    /// - `seg`: 图片消息段（必须是 type=image）
    ///
    /// # 返回
    /// - 图片二进制数据
    ///
    /// # 错误
    /// - 消息段不是图片类型
    /// - 缺少 file 字段
    /// - get_image API 调用失败
    /// - 本地文件读取失败或 URL 下载失败
    ///
    /// # 示例
    /// ```rust
    /// let img_seg = ctx.segments.iter().find(|s| s.is_image()).unwrap();
    /// let image_data = ctx.api.download_image_from_segment(img_seg).await?;
    /// ```
    pub async fn download_image_from_segment(&self, seg: &MessageSegment) -> Result<Vec<u8>> {
        if !seg.is_image() {
            anyhow::bail!("消息段不是图片类型");
        }

        // 提取 file_id
        let file_id = seg.image_file()
            .context("图片消息段缺少 file 字段")?;

        // 调用 get_image API
        let img_info = self.get_image(file_id).await?;

        // 优先使用本地文件
        if let Some(file_path) = img_info.file {
            return tokio::fs::read(&file_path).await
                .with_context(|| format!("读取本地图片失败: {}", file_path));
        }

        // 否则下载 URL
        if let Some(url) = img_info.url {
            return self.download_image_from_url(&url).await;
        }

        anyhow::bail!("get_image 返回的数据中既没有 file 也没有 url")
    }

    /// 从 URL 下载图片（使用 runtime/http 客户端）
    ///
    /// 内部方法，供 download_image_from_segment 使用。
    async fn download_image_from_url(&self, url: &str) -> Result<Vec<u8>> {
        #[cfg(feature = "runtime-http")]
        let client = crate::runtime::http::client();
        #[cfg(not(feature = "runtime-http"))]
        let client = {
            static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .expect("构建 HTTP 客户端失败")
            })
        };

        let resp = client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .with_context(|| format!("下载图片失败: {}", url))?;

        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }
}
