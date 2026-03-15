use serde::Deserialize;
use time::format_description::FormatItem;
use time::macros::format_description;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

#[cfg(feature = "core-log-file")]
use std::io::Write;
#[cfg(feature = "core-log-file")]
use std::path::PathBuf;
#[cfg(feature = "core-log-file")]
use std::sync::Mutex;

// ── 日志配置 ──────────────────────────────────────────────────────────────────

/// runtime.toml `[log]` 段。
#[derive(Debug, Deserialize)]
pub struct LogConfig {
    /// 日志文件目录 — 仅编译时启用 core-log-file 后生效
    #[cfg(feature = "core-log-file")]
    pub log_dir: Option<String>,
    /// 保留天数（启动时清理超期日志文件），默认 30 — 仅 core-log-file
    #[cfg(feature = "core-log-file")]
    #[serde(default = "LogConfig::default_max_days")]
    pub max_days: u32,
    /// 日志级别（trace/debug/info/warn/error），默认 info
    #[serde(default = "LogConfig::default_level")]
    pub level: String,
}

impl LogConfig {
    fn default_level() -> String { "info".to_string() }
    #[cfg(feature = "core-log-file")]
    fn default_max_days() -> u32 { 30 }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "core-log-file")]
            log_dir:  None,
            #[cfg(feature = "core-log-file")]
            max_days: 30,
            level:    Self::default_level(),
        }
    }
}

/// 按配置时区格式化的日志计时器。
fn configured_timer() -> OffsetTime<&'static [FormatItem<'static>]> {
    OffsetTime::new(
        crate::runtime::time::utc_offset(),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3] [offset_hour sign:mandatory]:[offset_minute padding:zero]"),
    )
}

/// 不透明的日志 Guard，持有至 `main()` 结束以确保所有日志刷盘。
/// 未启用 `core-log-file` 时为零大小占位类型，无运行时开销。
#[cfg(feature = "core-log-file")]
pub use tracing_appender::non_blocking::WorkerGuard as LogGuard;

#[cfg(not(feature = "core-log-file"))]
pub struct LogGuard;

/// 初始化 tracing。
///
/// - 始终输出到 stdout（供 journald 收集）
/// - 编译启用 `core-log-file` 且 `cfg.log_dir` 有值时，额外写入每日滚动
///   日志文件 `<log_dir>/lianbot.log.<YYYY-MM-DD>`，并在启动时清理超期文件。
///
/// 返回 `Some(LogGuard)` 表示开启了文件日志，调用方须将其保存至
/// `main()` 生命周期结束，否则最后一批日志可能丢失。
pub fn init(cfg: &LogConfig) -> Option<LogGuard> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    let stdout_layer = fmt::layer().with_writer(std::io::stdout).with_timer(configured_timer());

    // 文件日志层（仅 core-log-file feature 编译进来）
    #[cfg(feature = "core-log-file")]
    if let Some(dir) = &cfg.log_dir {
        use std::fs;
        use std::time::{Duration, SystemTime};

        // 清理超期旧文件
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(u64::from(cfg.max_days) * 86400))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if !entry.file_name().to_string_lossy().starts_with("lianbot.log.") { continue; }
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if mtime < cutoff {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }

        match fs::create_dir_all(dir) {
            Err(e) => eprintln!("[logger] 无法创建日志目录 {dir}: {e}，回退到纯 stdout"),
            Ok(()) => {
                // 写入前检测目录是否真的可写，避免 tracing-appender panic
                let probe = std::path::Path::new(dir).join(".lianbot_write_probe");
                match fs::write(&probe, b"") {
                    Err(e) => {
                        eprintln!("[logger] 日志目录不可写 {dir}: {e}，回退到纯 stdout");
                    }
                    Ok(()) => {
                        let _ = fs::remove_file(&probe);

                        // 注意：tracing_appender::rolling::daily() 使用 UTC 时间来决定日期，
                        // 无法使用我们配置的时区（UTC+8）。因此使用自定义滚动器，
                        // 它调用 runtime::time::now() 来获取配置时区的当前日期。
                        let file_appender = ConfiguredRollingAppender::new(dir, "lianbot.log");
                        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                        let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false).with_timer(configured_timer());
                        tracing_subscriber::registry()
                            .with(filter)
                            .with(stdout_layer)
                            .with(file_layer)
                            .init();
                        return Some(guard);
                    }
                }
            }
        }
    }

    // 纯 stdout（feature 未启用 / log_dir 未设置 / 目录创建失败）
    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .init();
    None
}

// ── 使用配置时区的日志滚动器 ──────────────────────────────────────────────────

/// 自定义日志滚动器，使用 runtime::time::now() 获取配置时区的日期。
///
/// tracing_appender::rolling::daily() 使用 UTC 时间，无法使用我们配置的时区。
/// 这个实现在每次写入时检查日期，如果日期变化则滚动到新文件。
#[cfg(feature = "core-log-file")]
struct ConfiguredRollingAppender {
    dir: PathBuf,
    prefix: String,
    current_file: Mutex<Option<(String, std::fs::File)>>,
}

#[cfg(feature = "core-log-file")]
impl ConfiguredRollingAppender {
    fn new(dir: impl Into<PathBuf>, prefix: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            prefix: prefix.into(),
            current_file: Mutex::new(None),
        }
    }

    /// 获取配置时区的当前日期（YYYY-MM-DD 格式）
    fn current_date() -> String {
        use chrono::Datelike;
        let now = crate::runtime::time::now();
        format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
    }

    fn get_writer(&self) -> std::io::Result<std::sync::MutexGuard<'_, Option<(String, std::fs::File)>>> {
        let mut guard = self.current_file.lock().unwrap();
        let today = Self::current_date();

        // 检查是否需要滚动（日期变化或首次写入）
        let needs_rotation = match &*guard {
            None => true,
            Some((date, _)) => date != &today,
        };

        if needs_rotation {
            let filename = format!("{}.{}", self.prefix, today);
            let path = self.dir.join(&filename);
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            *guard = Some((today, file));
        }

        Ok(guard)
    }
}

#[cfg(feature = "core-log-file")]
impl Write for ConfiguredRollingAppender {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = self.get_writer()?;
        if let Some((_, file)) = &mut *guard {
            file.write(buf)
        } else {
            Ok(0)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut guard = self.current_file.lock().unwrap();
        if let Some((_, file)) = &mut *guard {
            file.flush()
        } else {
            Ok(())
        }
    }
}
