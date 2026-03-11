//! LLM Tool-Call 支持
//!
//! 将 CommandRegistry 中声明了 `tool_description()` 的命令
//! 转换为 system prompt 片段，并解析 LLM 的 tool-call 响应。

use serde::Deserialize;
use tracing::debug;

// ── LLM 响应结构 ─────────────────────────────────────────────────────────────

/// LLM 在 tool-call 模式下返回的 JSON 结构
#[derive(Debug, Deserialize)]
struct LlmToolResponse {
    /// "chat" 表示普通回复，"tool" 表示调用命令
    #[serde(rename = "type")]
    response_type: String,
    /// 普通回复时的文本内容
    content: Option<String>,
    /// tool-call 时的命令名
    command: Option<String>,
    /// tool-call 时附带的消息（可选，发送给用户的过渡语）
    message: Option<String>,
}

/// 解析结果
pub enum ParsedResponse {
    /// LLM 选择普通文字回复
    Chat(String),
    /// LLM 选择调用一个命令
    ToolCall {
        command: String,
        message: Option<String>,
    },
}

// ── System Prompt 工具段 ─────────────────────────────────────────────────────

/// 根据 tool 定义列表，生成追加到 system prompt 的工具说明段。
pub fn build_tools_prompt(tools: &[(&str, &str)]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let mut prompt = String::from(
        "\n\n== 可用工具 ==\n\
         你拥有以下工具（Bot 命令），可以在合适的时候调用它们来帮助用户：\n",
    );

    for (name, desc) in tools {
        prompt.push_str(&format!("- {name}: {desc}\n"));
    }

    prompt.push_str(
        "\n== 回复格式（重要！）==\n\
         你必须严格按照以下 JSON 格式回复，不要输出任何 JSON 以外的内容：\n\n\
         普通回复（不需要调用工具时）：\n\
         {\"type\": \"chat\", \"content\": \"你的回复内容\"}\n\n\
         调用工具（需要执行某个命令时）：\n\
         {\"type\": \"tool\", \"command\": \"命令名\", \"message\": \"给用户的过渡语（可选）\"}\n\n\
         注意：\n\
         - 只在用户的意图明确需要某个工具时才调用，日常闲聊直接用 chat 类型回复。\n\
         - command 必须是上面列出的工具名之一。\n\
         - message 是可选的，用于在执行命令前给用户一个自然的过渡（如\"让我看看~\"），可以省略。\n\
         - content 中的换行用 \\n 表示。\n",
    );

    prompt
}

/// 解析 LLM 返回的 JSON 响应。
/// 如果 JSON 解析失败，将整个文本视为普通回复（兼容不支持 JSON 的模型）。
pub fn parse_response(raw: &str) -> ParsedResponse {
    let trimmed = raw.trim();

    // 尝试解析 JSON
    if let Ok(resp) = serde_json::from_str::<LlmToolResponse>(trimmed) {
        match resp.response_type.as_str() {
            "tool" => {
                if let Some(cmd) = resp.command.filter(|c| !c.is_empty()) {
                    debug!("[chat/tools] LLM tool-call: command={cmd}");
                    return ParsedResponse::ToolCall {
                        command: cmd,
                        message: resp.message.filter(|m| !m.is_empty()),
                    };
                }
                // command 为空，当作普通回复
                debug!("[chat/tools] LLM 返回 tool 但无 command，降级为 chat");
            }
            "chat" => {
                if let Some(content) = resp.content.filter(|c| !c.is_empty()) {
                    return ParsedResponse::Chat(content);
                }
            }
            other => {
                debug!("[chat/tools] 未知 response type: {other}，降级为纯文本");
            }
        }
    }

    // JSON 解析失败或结构不符 → 整段文本当做普通回复
    debug!("[chat/tools] 非 JSON 响应，按纯文本处理");
    ParsedResponse::Chat(trimmed.to_string())
}
