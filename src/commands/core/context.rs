use std::{collections::HashMap, hash::{BuildHasher, Hasher, RandomState}, sync::Arc};

use anyhow::{Context, Result};
use tokio::sync::Mutex;

#[cfg(feature = "runtime-permission")]
use crate::runtime::permission::{AccessControl, BotUser, Scope};

#[cfg(feature = "runtime-api")]
use crate::runtime::api::{ApiClient, MsgTarget};

#[cfg(feature = "runtime-parser")]
use crate::runtime::parser::ParamValue;

#[cfg(feature = "runtime-pool")]
use crate::runtime::pool::Pool;

#[cfg(feature = "runtime-registry")]
use crate::runtime::registry::CommandRegistry;

#[cfg(feature = "runtime-typ")]
use crate::runtime::typ::MessageSegment;

#[cfg(feature = "runtime-ws")]
use crate::runtime::ws::WsManager;

/// 命令调用来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// 用户直接调用：需要友好提示，输出给用户
    User,
    /// LLM Tool Call：返回结构化数据，不输出给用户
    ToolCall,
}

pub struct CommandContext {
    /// 本次命令执行的追踪标识（8 字符 hex），用于关联并发日志
    pub trace_id: String,
    /// 触发消息的 message_id（预留字段，用于未来可能的消息引用功能如撤回、回复等）
    /// 注：当前版本暂未使用，但保留以便后续扩展
    #[allow(dead_code)]
    pub message_id: Option<i64>,
    /// 发送者的虚拟用户对象（包含 user_id、scope、role）
    pub bot_user: BotUser,
    /// 原始消息段列表（含图片/at/回复等非文本 segment）
    pub segments: Vec<MessageSegment>,
    /// 解析后的参数 map
    pub params: HashMap<String, ParamValue>,
    /// OneBot API 客户端（Arc 共享）
    pub api: Arc<ApiClient>,
    /// WebSocket 连接管理器（Arc 共享）
    #[cfg(feature = "runtime-ws")]
    pub ws: Option<Arc<WsManager>>,
    /// 命令前缀（从 runtime 配置提取）
    pub cmd_prefix: String,
    /// 命令注册表（供 /help 等命令枚举全部命令）
    pub registry: Arc<CommandRegistry>,
    /// 消息池（per-scope 内存缓冲，可选）
    #[allow(dead_code)]
    #[cfg(feature = "runtime-pool")]
    pub pool: Option<Arc<Pool>>,
    /// 准入控制（block/unblock、enable/disable 等管理操作）
    pub access: Arc<AccessControl>,
    /// 调用来源
    pub invocation: Invocation,
    /// 捕获的输出（ToolCall 模式下使用）
    pub(crate) captured_output: Arc<Mutex<Option<String>>>,
}

impl CommandContext {
    // ── Scope 便捷方法 ────────────────────────────────────────────────────────

    /// 当前交互域的发送目标（自动从 bot_user.scope 派生）。
    fn target(&self) -> MsgTarget {
        MsgTarget::from(self.bot_user.scope)
    }

    /// 获取当前交互域（Scope）。
    ///
    /// **注意：** 优先使用 `group_id()` 等专用方法。
    /// 只有在确实需要完整 `Scope` 对象时才使用此方法（如传递给其他模块）。
    pub fn scope(&self) -> Scope {
        self.bot_user.scope
    }

    /// 若当前 scope 是群聊，返回 group_id；否则 None。
    #[allow(dead_code)]
    pub fn group_id(&self) -> Option<i64> {
        match self.bot_user.scope {
            Scope::Group(gid) => Some(gid),
            _ => None,
        }
    }

    // ── 回复便捷方法 ─────────────────────────────────────────────────────────

    /// 向当前交互域发送纯文字回复。
    pub async fn reply(&self, text: &str) -> Result<()> {
        match self.invocation {
            Invocation::User => {
                // 直接发送给用户
                self.api.send_msg(self.target(), text).await
            }
            Invocation::ToolCall => {
                // 捕获到 buffer，返回给 LLM
                *self.captured_output.lock().await = Some(text.to_string());
                Ok(())
            }
        }
    }

    /// 向当前交互域发送图片。
    #[allow(dead_code)]
    pub async fn reply_image(&self, file: &str) -> Result<()> {
        self.api.send_image_to(self.target(), file).await
    }

    /// 向当前交互域发送文字 + 图片（同一条消息）。
    #[allow(dead_code)]
    pub async fn reply_text_image(&self, text: &str, file: &str) -> Result<()> {
        self.api.send_segments(self.target(), vec![
            MessageSegment::text(text),
            MessageSegment::image(file),
        ]).await
    }

    /// 向当前交互域发送任意消息段组合（text+image+text 等混合消息）。
    #[allow(dead_code)]
    pub async fn reply_segments(&self, segments: Vec<MessageSegment>) -> Result<()> {
        self.api.send_segments(self.target(), segments).await
    }

    /// 向当前交互域发送合并转发消息。
    /// `nodes` 由 `MessageSegment::node(...)` 构成。
    #[allow(dead_code)]
    pub async fn reply_forward(
        &self,
        nodes: Vec<MessageSegment>,
        source: Option<&str>,
        summary: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<()> {
        self.api.send_forward_msg(self.target(), nodes, source, summary, prompt).await
    }

    // ── 参数便捷方法 ─────────────────────────────────────────────────────────

    /// 按多个别名查找参数值字符串。
    /// 例如：`ctx.get(&["-u", "--url"])` 会按顺序尝试每个 key。
    pub fn get(&self, keys: &[&str]) -> Option<&str> {
        for &key in keys {
            if let Some(v) = self.params.get(key) {
                if let Some(s) = v.as_str() {
                    return Some(s);
                }
            }
        }
        None
    }

    /// 解析 JSON 参数
    pub fn get_json<T: serde::de::DeserializeOwned>(&self, keys: &[&str]) -> Result<T> {
        for &key in keys {
            if let Some(v) = self.params.get(key) {
                if let Some(s) = v.as_str() {
                    return serde_json::from_str(s)
                        .with_context(|| format!("解析参数 {} 为 JSON 失败", key));
                }
            }
        }
        anyhow::bail!("未找到参数: {:?}", keys)
    }
}

/// 生成 8 字符十六进制随机 trace_id，纯标准库实现，不引入外部依赖。
pub fn gen_trace_id() -> String {
    let h = RandomState::new().build_hasher().finish();
    format!("{:08x}", h as u32)
}
