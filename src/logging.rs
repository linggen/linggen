use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::{fmt::time::ChronoUtc, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static FILTER_HANDLE: OnceLock<Handle<EnvFilter, Registry>> = OnceLock::new();

const DEFAULT_RETENTION_DAYS: u64 = 7;
const LOG_FILE_PREFIX: &str = "linggen";

pub struct LoggingSettings<'a> {
    pub level: Option<&'a str>,
    pub directory: Option<&'a str>,
    pub retention_days: Option<u64>,
    /// When true, suppress stdout logging (TUI mode owns the terminal).
    pub suppress_stdout: bool,
}

/// Build an `EnvFilter` for the given application log level.
fn build_filter(level: &str) -> EnvFilter {
    EnvFilter::try_new(format!(
        "ling={level},linggen_agent={level},linggen={level},\
         axum=warn,tower_http=warn,hyper=warn,hyper_util=warn,reqwest=warn,\
         mio=warn,reqwest_retry=warn"
    ))
    .unwrap_or_else(|_| EnvFilter::new("info"))
}

pub fn setup_tracing_with_settings(settings: LoggingSettings<'_>) -> Result<PathBuf> {
    let log_dir = resolve_log_dir(settings.directory)?;
    let retention_days = settings.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS).max(1);
    if let Err(e) = cleanup_old_logs(&log_dir, retention_days) {
        eprintln!("Failed to cleanup old logs: {e}");
    }

    let file_appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE_PREFIX);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    LOG_GUARD
        .set(guard)
        .map_err(|_| anyhow!(
            "Logging already initialized. Cannot setup logging multiple times."
        ))?;

    // Second-level timestamp precision to keep logs readable.
    let time_format = ChronoUtc::new("%Y-%m-%dT%H:%M:%S".to_string());

    // Stdout layer is None in TUI mode — ratatui owns the terminal.
    let stdout_layer = if settings.suppress_stdout {
        None
    } else {
        Some(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_file(true)
                .with_line_number(true)
                .with_level(true)
                .compact()
                .with_timer(time_format.clone()),
        )
    };

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact()
        .with_timer(time_format);

    // When level is explicitly set, override RUST_LOG; otherwise, use RUST_LOG first, then default
    let filter = if let Some(level) = settings.level {
        build_filter(level)
    } else {
        match EnvFilter::try_from_default_env() {
            Ok(env_filter) => env_filter,
            Err(_) => build_filter("info"),
        }
    };

    // Wrap the filter in a reload layer so we can change log level at runtime.
    let (reload_filter, handle) = tracing_subscriber::reload::Layer::new(filter);
    let _ = FILTER_HANDLE.set(handle);

    let _ = tracing_subscriber::registry()
        .with(reload_filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    Ok(log_dir)
}

/// Update the log level at runtime. Called when the user changes the level via settings.
pub fn set_log_level(level: &str) {
    let Some(handle) = FILTER_HANDLE.get() else {
        eprintln!("Cannot set log level: logging not initialized");
        return;
    };
    let new_filter = build_filter(level);
    if let Err(e) = handle.reload(new_filter) {
        eprintln!("Failed to reload log filter: {e}");
    }
}

fn resolve_log_dir(configured: Option<&str>) -> Result<PathBuf> {
    let dir = if let Some(path) = configured {
        expand_tilde(path)
    } else {
        crate::paths::logs_dir()
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn cleanup_old_logs(log_dir: &PathBuf, retention_days: u64) -> Result<()> {
    let now = SystemTime::now();
    let max_age = Duration::from_secs(60 * 60 * 24 * retention_days);
    for entry in std::fs::read_dir(log_dir)? {
        let entry = match entry {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to read directory entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(v) => v,
            None => continue,
        };
        if !file_name.starts_with(LOG_FILE_PREFIX) {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to get metadata for {:?}: {e}", path);
                continue;
            }
        };
        let age = match now.duration_since(modified) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to calculate age for {:?}: {e}", path);
                continue;
            }
        };
        // Use >= to remove files exactly at retention age
        if age >= max_age {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("Failed to remove old log file {:?}: {e}", path);
            }
        }
    }
    Ok(())
}
