use std::{collections::HashMap, sync::Arc};

use tracing::{info, warn};

use crate::commands::{Command, CommandKind};
use crate::runtime::pool::Pool;

#[cfg(feature = "runtime-ws")]
use crate::runtime::ws::WsManager;

// ── 命令注册表 ─────────────────────────────────────────────────────────────────
//
// 维护两张表：
//   simple_cmds   → 简单命令（kind = Simple），例如 "ping"
//   advanced_cmds → 复杂命令（kind = Advanced），例如 "smy"
//
// 所有命令名及别名均为纯名字（不含前缀），前缀由 parser 层统一处理。

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
    ///
    /// 在注册时检查命令的依赖是否满足，不满足的命令会被跳过并记录警告。
    #[cfg(feature = "runtime-ws")]
    pub fn register(
        &mut self,
        cmd: Arc<dyn Command>,
        pool: &Option<Arc<Pool>>,
        ws: &Option<Arc<WsManager>>,
    ) {
        // 依赖检查
        for dep in cmd.dependencies() {
            if !dep.is_available(pool, ws) {
                warn!(
                    "[registry] 跳过命令 {} - 缺少依赖: {}",
                    cmd.name(),
                    dep.description()
                );
                return;
            }
        }

        let name = cmd.name().to_string();
        let aliases = cmd.aliases().iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let kind_tag = match cmd.kind() {
            CommandKind::Simple   => "simple",
            CommandKind::Advanced => "advanced",
        };
        if aliases.is_empty() {
            info!("[registry] +{kind_tag} {name}");
        } else {
            info!("[registry] +{kind_tag} {name} (alias: {})", aliases.join(", "));
        }

        let table = match cmd.kind() {
            CommandKind::Simple   => &mut self.simple_cmds,
            CommandKind::Advanced => &mut self.advanced_cmds,
        };

        table.insert(name, cmd.clone());
        for alias in aliases {
            table.insert(alias, cmd.clone());
        }
    }

    #[cfg(not(feature = "runtime-ws"))]
    pub fn register(
        &mut self,
        cmd: Arc<dyn Command>,
        pool: &Option<Arc<Pool>>,
    ) {
        // 依赖检查
        for dep in cmd.dependencies() {
            if !dep.is_available(pool) {
                warn!(
                    "[registry] 跳过命令 {} - 缺少依赖: {}",
                    cmd.name(),
                    dep.description()
                );
                return;
            }
        }

        let name = cmd.name().to_string();
        let aliases = cmd.aliases().iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let kind_tag = match cmd.kind() {
            CommandKind::Simple   => "simple",
            CommandKind::Advanced => "advanced",
        };
        if aliases.is_empty() {
            info!("[registry] +{kind_tag} {name}");
        } else {
            info!("[registry] +{kind_tag} {name} (alias: {})", aliases.join(", "));
        }

        let table = match cmd.kind() {
            CommandKind::Simple   => &mut self.simple_cmds,
            CommandKind::Advanced => &mut self.advanced_cmds,
        };

        table.insert(name, cmd.clone());
        for alias in aliases {
            table.insert(alias, cmd.clone());
        }
    }

    /// 查找简单命令（纯名字，不含前缀）
    pub fn get_simple(&self, name: &str) -> Option<&Arc<dyn Command>> {
        self.simple_cmds.get(name)
    }

    /// 查找复杂命令（纯名字）
    pub fn get_advanced(&self, name: &str) -> Option<&Arc<dyn Command>> {
        self.advanced_cmds.get(name)
    }

    /// 收集所有声明了 `tool_description()` 的命令，返回 `(name, description)` 列表。
    /// 用于构造 LLM 的 tool-call system prompt。
    pub fn tool_definitions(&self) -> Vec<(&str, &str)> {
        let mut seen = std::collections::HashSet::new();
        let mut defs: Vec<(&str, &str)> = Vec::new();
        for cmd in self.simple_cmds.values().chain(self.advanced_cmds.values()) {
            if seen.insert(cmd.name()) {
                if let Some(desc) = cmd.tool_description() {
                    defs.push((cmd.name(), desc));
                }
            }
        }
        defs.sort_by_key(|(name, _)| *name);
        defs
    }

    /// 生成 /help 输出的完整命令列表文本。
    /// 按名称排序，别名重复条目自动去除，别名在名称旁展示。
    /// `prefix` 为简单命令的触发前缀（如 `"!!"`），会展示在帮助文本中。
    pub fn help_text(&self, prefix: &str) -> String {
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
            "── 简单命令 ──".to_string(),
        ];
        for cmd in &simple {
            let brief = cmd.help().lines().next().unwrap_or("");
            lines.push(format!("  {}{:<10}  {}", prefix, cmd.name(), brief));
        }
        lines.push("── 复杂命令（<名称> [参数]）──".to_string());
        for cmd in &advanced {
            let aliases = cmd.aliases();
            let name_part = if aliases.is_empty() {
                format!("<{}>", cmd.name())
            } else {
                format!("<{}> / <{}>", cmd.name(), aliases.join("> / <"))
            };
            lines.push(format!("  {:<18}  {}", name_part, cmd.help().lines().next().unwrap_or("")));
        }
        lines.push(String::new());
        lines.push("💡 输入 <命令> --help 查看详细参数".to_string());
        lines.join("\n")
    }
}
