use std::collections::HashMap;

use serde::Deserialize;

// ── 解析器配置 ────────────────────────────────────────────────────────────────

/// runtime.toml `[parser]` 段。
#[derive(Debug, Deserialize)]
pub struct ParserConfig {
    /// 简单命令前缀，默认 "!!"
    #[serde(default = "ParserConfig::default_prefix")]
    pub cmd_prefix: String,
}

impl ParserConfig {
    fn default_prefix() -> String { "!!".to_string() }
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self { cmd_prefix: Self::default_prefix() }
    }
}

// ── 解析结果 ──────────────────────────────────────────────────────────────────

/// 命令解析器的输出类型
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedCommand {
    /// 简单命令：`/ping`（`trailing` 保留名称后的所有 token，用于 `-h`/`--help` 拦截）
    Simple { name: String, trailing: Vec<String> },

    /// 复杂命令：`<img> -u https://... --scale=2`
    Advanced {
        name: String,
        params: HashMap<String, ParamValue>,
    },
}

/// 参数值类型（标志位 vs 字符串值）
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// 布尔标志：`-f` / `--force`
    Flag,
    /// 字符串值：`-u https://...` / `--count=100`
    Value(String),
}

impl ParamValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::Value(s) => Some(s.as_str()),
            ParamValue::Flag => None,
        }
    }

    #[allow(dead_code)]
    pub fn is_flag(&self) -> bool {
        matches!(self, ParamValue::Flag)
    }
}

// ── 解析器 ────────────────────────────────────────────────────────────────────

pub struct CommandParser;

impl CommandParser {
    /// 尝试将输入字符串解析为命令。
    /// `prefix` 为简单命令前缀（如 `"!!"`），`<>` 为复杂命令前缀（固定）。
    /// 返回的 name 为纯命令名（不含前缀）。
    pub fn parse(input: &str, prefix: &str) -> Option<ParsedCommand> {
        let s = input.trim();
        if s.starts_with(prefix) {
            Self::parse_simple(s, prefix)
        } else if s.starts_with('<') && s.contains('>') {
            Self::parse_advanced(s)
        } else {
            None
        }
    }

    /// 判断文本是否看起来像一条命令（快速前缀检查）
    #[allow(dead_code)]
    pub fn is_command(input: &str, prefix: &str) -> bool {
        let s = input.trim();
        s.starts_with(prefix) || (s.starts_with('<') && s.contains('>'))
    }

    // ── 内部：简单命令 ────────────────────────────────────────────────────────
    // 格式：`{prefix}name [trailing...]`
    // 返回的 name 不含前缀：`!!ping` → name = "ping"
    fn parse_simple(s: &str, prefix: &str) -> Option<ParsedCommand> {
        let rest = s.strip_prefix(prefix)?;
        let mut tokens = rest.split_whitespace();
        let name = tokens.next()?.to_string();
        if name.is_empty() {
            return None;
        }
        let trailing: Vec<String> = tokens.map(|t| t.to_string()).collect();
        Some(ParsedCommand::Simple { name, trailing })
    }

    // ── 内部：复杂命令 ────────────────────────────────────────────────────────
    // 格式：`<name> [-x val] [--key=val] [--key val] [-abc]`
    fn parse_advanced(s: &str) -> Option<ParsedCommand> {
        let close = s.find('>')?;
        let name = s[1..close].trim().to_string();
        if name.is_empty() {
            return None;
        }
        let rest = &s[close + 1..];
        let params = Self::parse_params(rest);
        Some(ParsedCommand::Advanced { name, params })
    }

    /// 解析参数段，支持：
    /// - `-x val`      → `{"-x": Value("val")}`
    /// - `-abc`        → `{"-a": Flag, "-b": Flag, "-c": Flag}`（连写短参）
    /// - `--key val`   → `{"--key": Value("val")}`
    /// - `--key=val`   → `{"--key": Value("val")}`
    /// - `-x=val`      → `{"-x": Value("val")}`
    /// - `--flag`      → `{"--flag": Flag}`
    pub fn parse_params(input: &str) -> HashMap<String, ParamValue> {
        let tokens = shell_split(input);
        let mut params: HashMap<String, ParamValue> = HashMap::new();
        let mut i = 0;

        while i < tokens.len() {
            let token = &tokens[i];

            // ① 长参数赋值  --key=value
            if token.starts_with("--") && token.contains('=') {
                let (key, val) = token.split_once('=').unwrap();
                params.insert(key.to_string(), ParamValue::Value(val.to_string()));
                i += 1;

            // ② 长参数  --key [value]
            } else if token.starts_with("--") {
                // 拒绝非法长参数（-- 或 --x 这种只有一个字母的）
                if token.len() <= 3 {
                    i += 1;
                    continue;
                }
                let next = tokens.get(i + 1);
                if let Some(v) = next.filter(|v| !v.starts_with('-')) {
                    params.insert(token.clone(), ParamValue::Value(v.clone()));
                    i += 2;
                } else {
                    params.insert(token.clone(), ParamValue::Flag);
                    i += 1;
                }

            // ③ 短参数赋值  -x=value
            } else if token.starts_with('-') && token.contains('=') && !token.starts_with("--") {
                let (key, val) = token.split_once('=').unwrap();
                params.insert(key.to_string(), ParamValue::Value(val.to_string()));
                i += 1;

            // ④ 连写短参数  -abc  →  -a, -b, -c 均为 Flag
            } else if token.starts_with('-')
                && !token.starts_with("--")
                && token.len() > 2
            {
                for ch in token[1..].chars() {
                    params.insert(format!("-{ch}"), ParamValue::Flag);
                }
                i += 1;

            // ⑤ 单短参数  -x [value]
            } else if token.starts_with('-') && !token.starts_with("--") {
                let next = tokens.get(i + 1);
                if let Some(v) = next.filter(|v| !v.starts_with('-')) {
                    params.insert(token.clone(), ParamValue::Value(v.clone()));
                    i += 2;
                } else {
                    params.insert(token.clone(), ParamValue::Flag);
                    i += 1;
                }

            // ⑥ 其他裸字符串（忽略）
            } else {
                i += 1;
            }
        }

        params
    }
}

// ── 简易 shell 风格分词 ───────────────────────────────────────────────────────
//
// 支持：双引号保留空格  "hello world" → 一个 token
// 不做 shell 转义（\n 等），保持简单

fn shell_split(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                // 不把引号本身加入 token
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ── 单元测试 ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let r = CommandParser::parse("!!ping", "!!").unwrap();
        assert_eq!(r, ParsedCommand::Simple { name: "ping".into(), trailing: vec![] });
    }

    #[test]
    fn test_simple_with_trailing() {
        let r = CommandParser::parse("!!help --help", "!!").unwrap();
        assert_eq!(r, ParsedCommand::Simple { name: "help".into(), trailing: vec!["--help".into()] });
    }

    #[test]
    fn test_custom_prefix() {
        let r = CommandParser::parse("/ping", "/").unwrap();
        assert_eq!(r, ParsedCommand::Simple { name: "ping".into(), trailing: vec![] });
    }

    #[test]
    fn test_advanced_basic() {
        let r = CommandParser::parse("<img> -u https://example.com/a.png", "!!").unwrap();
        match r {
            ParsedCommand::Advanced { name, params } => {
                assert_eq!(name, "img");
                assert_eq!(params["-u"], ParamValue::Value("https://example.com/a.png".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_advanced_long_eq() {
        let r = CommandParser::parse("<smy> --count=50", "!!").unwrap();
        match r {
            ParsedCommand::Advanced { name, params } => {
                assert_eq!(name, "smy");
                assert_eq!(params["--count"], ParamValue::Value("50".into()));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_multi_short_flags() {
        let params = CommandParser::parse_params("-abc");
        assert!(params.contains_key("-a"));
        assert!(params.contains_key("-b"));
        assert!(params.contains_key("-c"));
    }

    #[test]
    fn test_quoted_value() {
        let params = CommandParser::parse_params(r#"-m "hello world""#);
        assert_eq!(params["-m"], ParamValue::Value("hello world".into()));
    }

    #[test]
    fn test_not_command() {
        assert!(CommandParser::parse("普通消息", "!!").is_none());
        assert!(CommandParser::parse("", "!!").is_none());
        assert!(CommandParser::parse("/ping", "!!").is_none()); // wrong prefix
    }

    #[test]
    fn test_ai_flags() {
        let params = CommandParser::parse_params("-a --ai");
        assert_eq!(params.get("-a"), Some(&ParamValue::Flag));
        assert_eq!(params.get("--ai"), Some(&ParamValue::Flag));
    }
}
