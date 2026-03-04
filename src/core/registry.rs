use std::{collections::HashMap, sync::Arc};

use crate::commands::{Command, CommandKind};
use crate::commands;

// ── 命令注册表 ─────────────────────────────────────────────────────────────────
//
// 维护两张表：
//   simple_cmds  → 简单命令（kind = Simple），例如 "/ping"
//   advanced_cmds → 复杂命令（kind = Advanced），例如 "img"
//
// 命令的分类由 `cmd.kind()` 元数据决定，而非名称前缀。
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

    /// 注册一条命令（根据 `cmd.kind()` 元数据分类）。
    /// 同时注册别名。
    pub fn register(&mut self, cmd: Arc<dyn Command>) {
        let name = cmd.name().to_string();
        let aliases = cmd.aliases().iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let table = match cmd.kind() {
            CommandKind::Simple   => &mut self.simple_cmds,
            CommandKind::Advanced => &mut self.advanced_cmds,
        };

        table.insert(name, cmd.clone());
        for alias in aliases {
            table.insert(alias, cmd.clone());
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

    /// 生成 /help 输出的完整命令列表文本。
    /// 按名称排序，别名重复条目自动去除，别名在名称旁展示。
    pub fn help_text(&self) -> String {
        // 用 HashSet 按主名去重，避免别名条目重复出现
        let mut seen = std::collections::HashSet::new();
        let mut simple: Vec<&Arc<dyn Command>> = self.simple_cmds.values()
            .filter(|c| seen.insert(c.name()))
            .collect();
        simple.sort_by_key(|c| c.name());

        seen.clear();
        let mut advanced: Vec<&Arc<dyn Command>> = self.advanced_cmds.values()
            .filter(|c| seen.insert(c.name()))
            .collect();
        advanced.sort_by_key(|c| c.name());

        let mut lines = vec![
            "LianBot 命令列表".to_string(),
            "── 简单命令（/ 开头）──".to_string(),
        ];
        for cmd in &simple {
            lines.push(format!("  {:<10}  {}", cmd.name(), cmd.help()));
        }
        lines.push("── 复杂命令（<名称> [参数]）──".to_string());
        for cmd in &advanced {
            let aliases = cmd.aliases();
            let name_part = if aliases.is_empty() {
                format!("<{}>", cmd.name())
            } else {
                format!("<{}> / <{}>", cmd.name(), aliases.join("> / <"))
            };
            lines.push(format!("  {:<18}  {}", name_part, cmd.help()));
        }
        lines.push(String::new());
        lines.push("💡 输入 <命令> --help 查看详细参数".to_string());
        lines.join("\n")
    }
}

/// 构建并注册所有内置命令（仅注册已启用 feature 的命令）
impl Default for CommandRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        #[cfg(feature = "cmd-ping")]  registry.register(Arc::new(commands::ping::PingCommand));
        #[cfg(feature = "cmd-help")]  registry.register(Arc::new(commands::help::HelpCommand));
        #[cfg(feature = "cmd-img")]   registry.register(Arc::new(commands::img::ImgCommand));
        #[cfg(feature = "cmd-stalk")] registry.register(Arc::new(commands::stalk::StalkCommand));
        #[cfg(feature = "cmd-smy")]   registry.register(Arc::new(commands::smy::SmyCommand));
        #[cfg(feature = "cmd-alive")] registry.register(Arc::new(commands::alive::AliveCommand));
        #[cfg(feature = "cmd-world")] registry.register(Arc::new(commands::world::WorldCommand));
        registry
    }
}
