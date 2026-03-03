use std::{collections::HashMap, sync::Arc};

use super::{Command, commands};

// ── 命令注册表 ─────────────────────────────────────────────────────────────────
//
// 维护两张表：
//   simple_cmds  → 简单命令，key 含 '/'  例如 "/ping"
//   advanced_cmds → 复杂命令，key 不含 '/' 例如 "img"
//
// 调用 `CommandRegistry::default()` 会自动注册所有内置命令。

pub struct CommandRegistry {
    simple_cmds: HashMap<String, Arc<dyn Command>>,
    advanced_cmds: HashMap<String, Arc<dyn Command>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            simple_cmds: HashMap::new(),
            advanced_cmds: HashMap::new(),
        }
    }

    /// 注册一条命令（自动按前缀分类）。
    /// 同时注册别名。
    pub fn register(&mut self, cmd: Arc<dyn Command>) {
        let name = cmd.name().to_string();
        let aliases = cmd.aliases().iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let table = if name.starts_with('/') {
            &mut self.simple_cmds
        } else {
            &mut self.advanced_cmds
        };

        table.insert(name, cmd.clone());
        for alias in aliases {
            let t = if alias.starts_with('/') {
                &mut self.simple_cmds
            } else {
                &mut self.advanced_cmds
            };
            t.insert(alias, cmd.clone());
        }
    }

    /// 查找简单命令（key 含 '/'）
    pub fn get_simple(&self, name: &str) -> Option<&Arc<dyn Command>> {
        self.simple_cmds.get(name)
    }

    /// 查找复杂命令（key 不含 '/'）
    pub fn get_advanced(&self, name: &str) -> Option<&Arc<dyn Command>> {
        self.advanced_cmds.get(name)
    }

    /// 生成所有命令的帮助文本
    pub fn help_text(&self) -> String {
        let mut lines = vec!["── 简单命令 ──".to_string()];
        let mut simple: Vec<_> = self.simple_cmds.values().collect();
        simple.dedup_by_key(|c| c.name());
        for cmd in simple {
            lines.push(format!("  {}  {}", cmd.name(), cmd.help()));
        }
        lines.push("── 复杂命令 ──".to_string());
        let mut advanced: Vec<_> = self.advanced_cmds.values().collect();
        advanced.dedup_by_key(|c| c.name());
        for cmd in advanced {
            lines.push(format!("  <{}>  {}", cmd.name(), cmd.help()));
        }
        lines.join("\n")
    }
}

/// 构建并注册所有内置命令
impl Default for CommandRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(commands::ping::PingCommand));
        registry.register(Arc::new(commands::help::HelpCommand));
        registry.register(Arc::new(commands::img::ImgCommand));
        registry.register(Arc::new(commands::stalk::StalkCommand));
        registry.register(Arc::new(commands::smy::SmyCommand));
        registry.register(Arc::new(commands::alive::AliveCommand));
        registry
    }
}
