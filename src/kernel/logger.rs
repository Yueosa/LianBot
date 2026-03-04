use time::macros::{format_description, offset};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::kernel::config::LogConfig;

/// CST (+08:00) 时区计时器。
fn cst_timer() -> OffsetTime<&'static [time::format_description::FormatItem<'static>]> {
    OffsetTime::new(
        offset!(+8),
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]+08:00"),
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

    let stdout_layer = fmt::layer().with_writer(std::io::stdout).with_timer(cst_timer());

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
                if !entry.file_name().to_string_lossy().starts_with("lianbot.log.utc") { continue; }
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
                        let file_appender = tracing_appender::rolling::daily(dir, "lianbot.log.utc");
                        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                        let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false).with_timer(cst_timer());
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
