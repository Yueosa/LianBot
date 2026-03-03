use std::fs;
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::core::config::LogConfig;

/// 初始化 tracing。
///
/// - 始终输出到 stdout（供 journald 收集）
/// - 若 `cfg.log_dir` 有值，则额外写入每日滚动日志文件
///   `<log_dir>/lianbot.log.<YYYY-MM-DD>`；同时在启动时清理超期文件。
///
/// 返回 `Some(WorkerGuard)` 表示开启了文件日志，调用方须将其保存至
/// `main()` 生命周期结束，否则最后一批日志可能丢失。
pub fn init(cfg: &LogConfig) -> Option<WorkerGuard> {
    // EnvFilter：优先读 RUST_LOG，否则用配置文件中的 level
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    let stdout_layer = fmt::layer().with_writer(std::io::stdout);

    match &cfg.log_dir {
        Some(dir) => {
            // 清理超期旧文件
            cleanup_old_logs(dir, cfg.max_days);

            // 创建目录（不存在时）
            if let Err(e) = fs::create_dir_all(dir) {
                eprintln!("[logger] 无法创建日志目录 {dir}: {e}，回退到纯 stdout");
                tracing_subscriber::registry()
                    .with(filter)
                    .with(stdout_layer)
                    .init();
                return None;
            }

            let file_appender = tracing_appender::rolling::daily(dir, "lianbot.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false);

            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();

            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .init();
            None
        }
    }
}

/// 删除 `dir` 下文件名以 `lianbot.log.` 开头、修改时间早于 `max_days` 天的文件。
fn cleanup_old_logs(dir: &str, max_days: u32) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(max_days) * 86400))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // 目录不存在，无需清理
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("lianbot.log") {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if let Ok(mtime) = meta.modified() {
            if mtime < cutoff {
                if let Err(e) = fs::remove_file(entry.path()) {
                    eprintln!("[logger] 清理旧日志失败 {:?}: {e}", entry.path());
                }
            }
        }
    }
}
