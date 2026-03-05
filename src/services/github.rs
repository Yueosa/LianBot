//! GitHub Webhook 通知服务
//!
//! 数据流：
//!   POST /webhook/github（Axum handler）
//!     → HMAC 验签
//!     → `GitHubEvent` 放入 mpsc 通道
//!     → `GitHubService::run()` 消费、格式化、推送群消息

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::runtime::api::MsgTarget;

use super::{BotService, ServiceContext};

// ── 配置 ──────────────────────────────────────────────────────────────────────

/// plugins.toml 中 `[github]` 段
#[derive(Debug, Deserialize, Default)]
pub struct GitHubConfig {
    /// Webhook secret，与 GitHub 仓库设置里填写的一致
    #[serde(default)]
    pub secret: String,
    /// 订阅列表
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
}

/// 单条订阅规则
#[derive(Debug, Deserialize, Clone)]
pub struct Subscription {
    /// 指定仓库，格式 `"owner/repo"`（与 `user` 二选一）
    pub repo: Option<String>,
    /// 指定账号/组织下所有仓库，格式 `"owner"`（与 `repo` 二选一）
    pub user: Option<String>,
    /// 要监听的事件类型，如 `["push", "pull_request", "issues", "release"]`
    pub events: Vec<String>,
    /// 通知推送到哪个群
    pub group: i64,
    /// 通知时 @ 的 QQ 号列表（可为空）
    #[serde(default)]
    pub at: Vec<i64>,
}

impl Subscription {
    /// 判断该订阅是否匹配 `repo_full` ("owner/repo") 和 `event_type`
    pub fn matches(&self, repo_full: &str, event_type: &str) -> bool {
        let repo_match = if let Some(r) = &self.repo {
            r == repo_full
        } else if let Some(u) = &self.user {
            repo_full.starts_with(&format!("{u}/"))
        } else {
            false
        };
        let event_match = self.events.iter().any(|e| e == event_type || e == "*");
        repo_match && event_match
    }
}

// ── Channel 消息 ──────────────────────────────────────────────────────────────

/// Axum handler → GitHubService 通道消息
#[derive(Debug)]
pub struct GitHubEvent {
    /// X-GitHub-Event 头，如 "push" / "pull_request" / "issues" / "release"
    pub event_type: String,
    /// 仓库全名，如 "torvalds/linux"
    pub repo: String,
    /// 触发者登录名
    pub sender: String,
    /// 原始 JSON payload
    pub payload: serde_json::Value,
}

// ── HMAC 验签 ─────────────────────────────────────────────────────────────────

/// 验证 GitHub 签名头 `X-Hub-Signature-256: sha256=<hex>`
pub fn verify_signature(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let hex_part = match signature_header.strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };
    let sig_bytes = match hex::decode(hex_part) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}

// ── 消息格式化 ────────────────────────────────────────────────────────────────

fn format_event(evt: &GitHubEvent) -> Option<String> {
    let repo = &evt.repo;
    let sender = &evt.sender;
    let p = &evt.payload;

    match evt.event_type.as_str() {
        "push" => {
            let branch = p["ref"]
                .as_str()
                .unwrap_or("?")
                .trim_start_matches("refs/heads/");
            let commits: Vec<_> = p["commits"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .take(3)
                .filter_map(|c| {
                    let msg = c["message"].as_str().unwrap_or("").lines().next()?;
                    let id = &c["id"].as_str().unwrap_or("?")[..7.min(c["id"].as_str().unwrap_or("").len())];
                    Some(format!("  [{id}] {msg}"))
                })
                .collect();
            let total = p["commits"].as_array().map(|a| a.len()).unwrap_or(0);
            let mut text = format!(
                "📦 [{repo}] {sender} 向 {branch} 推送了 {total} 个提交"
            );
            if !commits.is_empty() {
                text.push('\n');
                text.push_str(&commits.join("\n"));
            }
            if total > 3 {
                text.push_str(&format!("\n  ... 共 {total} 个提交"));
            }
            Some(text)
        }

        "pull_request" => {
            let action = p["action"].as_str().unwrap_or("?");
            let number = p["number"].as_u64().unwrap_or(0);
            let title = p["pull_request"]["title"].as_str().unwrap_or("?");
            let url = p["pull_request"]["html_url"].as_str().unwrap_or("");
            match action {
                "opened" | "closed" | "reopened" | "ready_for_review" => {
                    let action_cn = match action {
                        "opened" => "新建",
                        "closed" => {
                            if p["pull_request"]["merged"].as_bool().unwrap_or(false) {
                                "已合并"
                            } else {
                                "已关闭"
                            }
                        }
                        "reopened" => "重新打开",
                        "ready_for_review" => "准备审查",
                        _ => action,
                    };
                    Some(format!(
                        "🔀 [{repo}] PR #{number} {action_cn}：{title}\n{url}"
                    ))
                }
                _ => None, // 忽略 labeled / assigned 等噪音事件
            }
        }

        "issues" => {
            let action = p["action"].as_str().unwrap_or("?");
            match action {
                "opened" | "closed" | "reopened" => {
                    let number = p["issue"]["number"].as_u64().unwrap_or(0);
                    let title = p["issue"]["title"].as_str().unwrap_or("?");
                    let url = p["issue"]["html_url"].as_str().unwrap_or("");
                    let action_cn = match action {
                        "opened" => "新建",
                        "closed" => "已关闭",
                        "reopened" => "重新打开",
                        _ => action,
                    };
                    Some(format!(
                        "📋 [{repo}] Issue #{number} {action_cn}：{title}\n{url}"
                    ))
                }
                _ => None,
            }
        }

        "release" => {
            let action = p["action"].as_str().unwrap_or("");
            if action != "published" {
                return None;
            }
            let tag = p["release"]["tag_name"].as_str().unwrap_or("?");
            let name = p["release"]["name"].as_str().unwrap_or(tag);
            let url = p["release"]["html_url"].as_str().unwrap_or("");
            Some(format!("🚀 [{repo}] 发布新版本 {tag}：{name}\n{url}"))
        }

        _ => {
            // 其余事件只记录日志，不推送
            info!("[github] 收到未处理事件: {} / {}", evt.event_type, repo);
            None
        }
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct GitHubService {
    rx: mpsc::Receiver<GitHubEvent>,
    ctx: ServiceContext,
    cfg: GitHubConfig,
}

impl GitHubService {
    pub fn new(rx: mpsc::Receiver<GitHubEvent>, ctx: ServiceContext, cfg: GitHubConfig) -> Self {
        Self { rx, ctx, cfg }
    }
}

impl BotService for GitHubService {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn run(mut self) -> anyhow::Result<()> {
        info!(
            "[{}] 已启动，共 {} 条订阅规则",
            self.name(),
            self.cfg.subscriptions.len()
        );

        while let Some(evt) = self.rx.recv().await {
            let Some(text) = format_event(&evt) else {
                continue;
            };

            // 找出所有匹配该事件的订阅
            let targets: Vec<(i64, Vec<i64>)> = self
                .cfg
                .subscriptions
                .iter()
                .filter(|s| s.matches(&evt.repo, &evt.event_type))
                .map(|s| (s.group, s.at.clone()))
                .collect();

            if targets.is_empty() {
                continue;
            }

            for (group_id, at_list) in targets {
                // 构造 @ 前缀
                let at_prefix: String = at_list
                    .iter()
                    .map(|qq| format!("@{qq} "))
                    .collect();
                let msg = format!("{at_prefix}{text}");

                if let Err(e) = self
                    .ctx
                    .api
                    .send_msg(MsgTarget::Group(group_id), &msg)
                    .await
                {
                    warn!("[github] 推送群 {group_id} 失败: {e:#}");
                }
            }
        }

        warn!("[{}] channel 已关闭，服务退出", self.name());
        Ok(())
    }
}
