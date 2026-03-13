//! 易班签到 Webhook 业务逻辑
//!
//! 接收 LianSign 脚本 POST 的签到概要，格式化为群消息。
//! 不依赖任何 runtime 模块，纯业务逻辑。

use serde::Deserialize;

// ── 配置 ──────────────────────────────────────────────────────────────────────

/// logic.toml 中 `[yiban]` 段
#[derive(Debug, Deserialize, Default)]
pub struct YiBanConfig {
    /// HMAC-SHA256 签名密钥，与 LianSign config.toml 中 webhook.secret 一致
    #[serde(default)]
    pub secret: String,
    /// LianSign HTTP 服务地址，如 http://127.0.0.1:9090
    #[serde(default)]
    pub api_url: String,
    /// LianSign HTTP 服务的 Bearer token
    #[serde(default)]
    pub api_token: String,
    /// 推送目标列表（对齐 GitHub subscriptions 模式）
    #[serde(default)]
    pub targets: Vec<YiBanTarget>,
    /// 命令触发后是否回源推送结果到触发来源，默认 true
    #[serde(default = "YiBanConfig::default_reply_origin")]
    pub reply_origin: bool,
    /// 每日自动签到时间（0-23 小时），None 时关闭定时签到
    #[serde(default)]
    pub auto_sign_hour: Option<u8>,
}

impl YiBanConfig {
    fn default_reply_origin() -> bool { true }
}

/// 单条推送目标规则
#[derive(Debug, Deserialize, Clone)]
pub struct YiBanTarget {
    /// 按用户名过滤（空列表 = 匹配所有用户）
    #[serde(default)]
    pub users: Vec<String>,
    /// 推送到哪个群
    pub group: i64,
    /// 通知时 @ 的 QQ 号列表
    #[serde(default)]
    pub at: Vec<i64>,
}

impl YiBanTarget {
    /// 判断该 target 是否匹配报告中的任何用户。
    /// `users` 为空时匹配所有。
    pub fn matches_any(&self, user_names: &[&str]) -> bool {
        self.users.is_empty()
            || self.users.iter().any(|u| user_names.contains(&u.as_str()))
    }
}

// ── Webhook 数据模型 ──────────────────────────────────────────────────────────

/// LianSign 脚本 POST 来的签到概要
#[derive(Debug, Deserialize)]
pub struct YiBanReport {
    /// 签到开始时间
    pub time: String,
    /// 耗时（秒）
    pub elapsed: u64,
    /// 每个用户的签到结果
    pub users: Vec<UserResult>,
}

/// 单个用户的签到结果
#[derive(Debug, Deserialize)]
pub struct UserResult {
    /// 用户昵称
    pub name: String,
    /// 状态：成功 / 登录失败 / 无任务 / 部分失败 / 已禁用 / 崩溃
    pub status: String,
    /// 处理的任务列表
    #[serde(default)]
    pub tasks: Vec<TaskResult>,
    /// 错误详情（仅在失败时存在）
    #[serde(default)]
    pub error_msg: Option<String>,
}

/// 单个任务的提交结果
#[derive(Debug, Deserialize)]
pub struct TaskResult {
    /// 任务标题
    pub title: String,
    /// 是否成功
    pub ok: bool,
}

// ── 消息格式化 ────────────────────────────────────────────────────────────────

/// 将签到概要格式化为群消息文本
pub fn format_report(report: &YiBanReport) -> String {
    let mut text = format!("📋 易班签到报告  {}\n", report.time);
    text.push_str(&format!("⏱ 耗时 {} 秒\n", report.elapsed));
    text.push_str("─────────────────\n");

    for user in &report.users {
        let icon = match user.status.as_str() {
            "成功" => "✅",
            "无任务" => "📭",
            "已禁用" => "⏸",
            "登录失败" => "❌",
            "部分失败" => "⚠️",
            "崩溃" => "💥",
            _ => "❓",
        };
        text.push_str(&format!("{icon} {}: {}\n", user.name, user.status));
        if let Some(ref msg) = user.error_msg {
            text.push_str(&format!("  └ {}\n", msg));
        }
        for task in &user.tasks {
            let t_icon = if task.ok { "  ✓" } else { "  ✗" };
            text.push_str(&format!("{t_icon} {}\n", task.title));
        }
    }

    text
}
