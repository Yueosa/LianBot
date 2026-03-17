//! LLM Tool-Call 支持
//!
//! 将 CommandRegistry 中声明了 `tool_description()` 的命令
//! 转换为 system prompt 片段，并解析 LLM 的 tool-call 响应。

use serde::Deserialize;
use tracing::debug;
use std::collections::HashMap;

// ── LLM 响应结构 ─────────────────────────────────────────────────────────────

/// LLM 响应类型
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseType {
    /// 调用命令，结果返回给 LLM，继续推理
    ToolCall,
    /// 调用命令，输出直接发送给用户，结束对话
    ToolCallEnd,
    /// 直接回答用户，结束对话
    EndText,
}

/// LLM 在 tool-call 模式下返回的 JSON 结构
#[derive(Debug, Deserialize)]
struct LlmToolResponse {
    #[serde(rename = "type")]
    response_type: ResponseType,

    // tool_call / tool_call_end 时使用
    command: Option<String>,
    params: Option<HashMap<String, String>>,

    // end_text 时使用
    content: Option<String>,
}

/// 解析结果
pub enum ParsedResponse {
    /// LLM 调用命令，继续推理
    ToolCall {
        command: String,
        params: HashMap<String, String>,
    },
    /// LLM 调用命令并结束
    ToolCallEnd {
        command: String,
        params: HashMap<String, String>,
    },
    /// LLM 直接回答并结束
    EndText(String),
}

// ── System Prompt 工具段 ─────────────────────────────────────────────────────

/// 根据 tool 定义列表，生成追加到 system prompt 的工具说明段。
pub fn build_tools_prompt(tools: &[(&str, &str)], current_round: usize, max_rounds: usize) -> String {
    if tools.is_empty() {
        return String::new();
    }

    format!(
        r#"
== 当前状态 ==
- 当前轮数：{current_round}/{max_rounds}
- 剩余轮数：{remaining}

== 可用工具 ==
你拥有以下工具（Bot 命令），可以在合适的时候调用它们来帮助用户：
{tools_list}

== 回复格式（重要！）==
你必须严格按照以下 JSON 格式回复，不要输出任何 JSON 以外的内容：

1. 调用工具继续推理（结果会返回给你，用户看不到）：
{{"type": "tool_call", "command": "命令名", "params": {{}}}}

2. 调用工具并结束（命令输出直接发送给用户，你不会再收到结果）：
{{"type": "tool_call_end", "command": "命令名", "params": {{}}}}

3. 直接回答并结束：
{{"type": "end_text", "content": "你的回复内容"}}

== 使用场景说明 ==

**tool_call（继续推理）**：
- 用于收集信息，结果会返回给你，用户看不到
- 适用场景：
  * 需要先识别图片内容，再用自然语言回答
  * 需要查询设备状态，再综合回答
  * 需要多个信息源综合判断
- 示例：用户问"这是什么？"[图片] → 你调用 vision 获取描述 → 你用自然语言回答

**tool_call_end（直接展示命令结果）**：
- 命令输出直接发送给用户，你不会再收到结果，对话结束
- 适用场景：
  * 用户明确要求执行某个命令（如"来张二次元图"）
  * 命令的输出就是最终答案
- 示例：用户说"来张二次元图" → 你调用 acg → 图片直接发给用户

**end_text（自然语言回答）**：
- 你直接用自然语言回答用户，不调用任何命令
- 适用场景：
  * 已经通过 tool_call 收集了足够信息
  * 用户问题不需要调用命令
  * 纯聊天对话

== 重要原则 ==
1. **不要试探性调用**：不要用 tool_call 来"看看命令会输出什么"，如果你想让用户看到命令结果，直接用 tool_call_end
2. **明确目的**：每次 tool_call 都应该有明确的信息收集目的
3. **轮数限制**：你最多有 {max_rounds} 轮，当前是第 {current_round} 轮，请合理规划
4. **及时结束**：收集到足够信息后，应该用 end_text 或 tool_call_end 结束对话，不要无意义地继续调用

== 示例 ==

场景 1：识图后回答（2 轮）
用户: [@Bot] 这是什么？[图片]
第 1 轮你: {{"type": "tool_call", "command": "vision"}}
系统返回: "一只橘猫躺在沙发上"
第 2 轮你: {{"type": "end_text", "content": "这是一只可爱的橘猫，正舒服地躺在沙发上休息呢~"}}

场景 2：直接执行命令（1 轮）
用户: [@Bot] 来张二次元图
第 1 轮你: {{"type": "tool_call_end", "command": "acg"}}
（图片直接发给用户，结束）
"#,
        current_round = current_round,
        max_rounds = max_rounds,
        remaining = max_rounds - current_round,
        tools_list = tools
            .iter()
            .map(|(name, desc)| format!("- {}: {}", name, desc))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// 解析 LLM 返回的 JSON 响应。
/// 如果 JSON 解析失败，返回错误（不再降级为 Chat）。
pub fn parse_response(raw: &str) -> Result<ParsedResponse, String> {
    let trimmed = raw.trim();

    // 尝试解析 JSON
    let resp: LlmToolResponse = serde_json::from_str(trimmed)
        .map_err(|e| format!("JSON 解析失败: {}", e))?;

    match resp.response_type {
        ResponseType::ToolCall => {
            let command = resp.command
                .filter(|c| !c.is_empty())
                .ok_or_else(|| "tool_call 缺少 command 字段".to_string())?;
            let params = resp.params.unwrap_or_default();
            debug!("[chat/tools] LLM tool_call: command={}", command);
            Ok(ParsedResponse::ToolCall { command, params })
        }
        ResponseType::ToolCallEnd => {
            let command = resp.command
                .filter(|c| !c.is_empty())
                .ok_or_else(|| "tool_call_end 缺少 command 字段".to_string())?;
            let params = resp.params.unwrap_or_default();
            debug!("[chat/tools] LLM tool_call_end: command={}", command);
            Ok(ParsedResponse::ToolCallEnd { command, params })
        }
        ResponseType::EndText => {
            let content = resp.content
                .filter(|c| !c.is_empty())
                .ok_or_else(|| "end_text 缺少 content 字段".to_string())?;
            debug!("[chat/tools] LLM end_text: {} chars", content.len());
            Ok(ParsedResponse::EndText(content))
        }
    }
}
