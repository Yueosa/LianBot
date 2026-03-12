use anyhow::{bail, Result};
use async_trait::async_trait;
use rand::seq::SliceRandom;
use regex::Regex;
use serde::Deserialize;
use tracing::debug;

use crate::commands::{Command, CommandContext, CommandKind, http_client};
use crate::runtime::typ::MessageSegment;

const OWNER: &str = "Cute-Dress";
const REPO: &str = "Dress";
const BRANCH: &str = "master";
const REPO_URL: &str = "https://github.com/Cute-Dress/Dress/";
const LICENSE: &str = "CC BY-NC-SA 4.0";

const IMAGE_EXTS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".webp",
    ".tiff", ".tif", ".heic", ".heif",
    ".avif", ".gif",
];

#[derive(Deserialize)]
struct TreeResponse {
    tree: Vec<TreeItem>,
}

#[derive(Deserialize)]
struct TreeItem {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

fn is_image(path: &str) -> bool {
    let lower = path.to_lowercase();
    IMAGE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

pub struct DressCommand;

#[async_trait]
impl Command for DressCommand {
    fn name(&self) -> &str { "dress" }
    fn help(&self) -> &str { "随机女装图片（来自 Cute-Dress/Dress 仓库）" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }
    fn tool_description(&self) -> Option<&str> {
        Some("从 GitHub Cute-Dress/Dress 开源女装仓库随机抽取一张图片发送，附带作者名和仓库信息。适合用户想看图、找乐子、或提到女装时调用")
    }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{OWNER}/{REPO}/git/trees/{BRANCH}?recursive=1"
        );

        let resp = match http_client()
            .get(&url)
            .header("User-Agent", "LianBot")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ctx.reply(&format!("❌ 请求 GitHub API 失败: {e}")).await,
        };

        let tree: TreeResponse = match resp.json().await {
            Ok(t) => t,
            Err(e) => return ctx.reply(&format!("❌ 解析 GitHub 响应失败: {e}")).await,
        };

        let images: Vec<&TreeItem> = tree
            .tree
            .iter()
            .filter(|item| item.kind == "blob" && is_image(&item.path))
            .collect();

        if images.is_empty() {
            return ctx.reply("❌ 仓库中未找到图片").await;
        }

        // 筛选出有英文作者名目录的图片（path 格式: 顶级目录/英文作者名/...）
        let author_re = Regex::new(r"^[A-Za-z][A-Za-z0-9_-]*$").expect("regex");
        let with_author: Vec<(&str, &str)> = images
            .iter()
            .filter_map(|item| {
                let parts: Vec<&str> = item.path.split('/').collect();
                if parts.len() > 1 && author_re.is_match(parts[1]) {
                    Some((parts[1], item.path.as_str()))
                } else {
                    None
                }
            })
            .collect();

        let (author, path) = if !with_author.is_empty() {
            let mut rng = rand::thread_rng();
            let &(author, path) = with_author.choose(&mut rng).unwrap();
            (author.to_string(), path.to_string())
        } else {
            // fallback: 随机选任意图片
            let mut rng = rand::thread_rng();
            let item = images.choose(&mut rng).unwrap();
            ("unknown".to_string(), item.path.clone())
        };

        let encoded_path = path
            .split('/')
            .map(|seg| urlencoding::encode(seg))
            .collect::<Vec<_>>()
            .join("/");
        let raw_url = format!(
            "https://raw.githubusercontent.com/{OWNER}/{REPO}/{BRANCH}/{encoded_path}"
        );

        debug!("[dress] author={author}, url={raw_url}");

        ctx.reply_segments(vec![
            MessageSegment::text(&format!("作者: {author}\n")),
            MessageSegment::image(&raw_url),
            MessageSegment::text(&format!("\n源仓库: {REPO_URL}\n开源协议: {LICENSE}")),
        ])
        .await
    }
}
