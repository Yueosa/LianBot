//! GitHub Webhook 业务逻辑
//!
//! 包含配置模型、订阅匹配、事件格式化。
//! 不依赖任何 runtime 模块，纯业务逻辑。
//! HMAC 验签已迁移至 `runtime::webhook::verify_hmac_sha256`。

use serde::Deserialize;
use tracing::info;

// ── 配置 ──────────────────────────────────────────────────────────────────────

/// logic.toml 中 `[github]` 段
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

// ── 事件数据 ──────────────────────────────────────────────────────────────────

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

// ── 消息格式化 ────────────────────────────────────────────────────────────────

/// 将 GitHub webhook 事件格式化为群消息文本。
/// 返回 `None` 表示该事件不需要推送。
pub fn format_event(evt: &GitHubEvent) -> Option<String> {
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
                _ => None,
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

        "star" => {
            let action = p["action"].as_str().unwrap_or("?");
            let count = p["repository"]["stargazers_count"].as_u64().unwrap_or(0);
            match action {
                "created" => Some(format!("⭐ [{repo}] {sender} starred（共 {count} stars）")),
                "deleted" => Some(format!("💔 [{repo}] {sender} unstarred（共 {count} stars）")),
                _ => None,
            }
        }

        "fork" => {
            let fork_name = p["forkee"]["full_name"].as_str().unwrap_or("?");
            let url = p["forkee"]["html_url"].as_str().unwrap_or("");
            Some(format!("🍴 [{repo}] {sender} fork → {fork_name}\n{url}"))
        }

        "issue_comment" => {
            let action = p["action"].as_str().unwrap_or("?");
            if action != "created" {
                return None;
            }
            let number = p["issue"]["number"].as_u64().unwrap_or(0);
            let title = p["issue"]["title"].as_str().unwrap_or("?");
            let body = p["comment"]["body"].as_str().unwrap_or("");
            let url = p["comment"]["html_url"].as_str().unwrap_or("");
            let is_pr = p["issue"]["pull_request"].is_object();
            let icon = if is_pr { "🔀" } else { "📋" };
            let kind = if is_pr { "PR" } else { "Issue" };
            let preview: String = body.lines().next().unwrap_or("").chars().take(80).collect();
            Some(format!(
                "💬 [{repo}] {sender} 评论了 {icon} {kind} #{number}：{title}\n{preview}\n{url}"
            ))
        }

        _ => {
            let action = p["action"].as_str().unwrap_or("");
            let hint = if action.is_empty() {
                format!("📡 [{repo}] 收到 {} 事件（by {sender}）", evt.event_type)
            } else {
                format!("📡 [{repo}] 收到 {}/{action} 事件（by {sender}）", evt.event_type)
            };
            Some(hint)
        }
    }
}
