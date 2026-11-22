//! Logging with `tracing`. A file under `<data>/logs/` always; stderr only with `-v`.
//!
//! Records carry structured fields (`repo`, `seq`, `oid`, `path`) so `cartograph report` can drop
//! paths. Logs never hold file contents.
//!
//! tracing-appender rotates by time, not size, so the proposed "7 files, 10 MB each" is approximated
//! as daily files with at most 7 kept.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Installs the global subscriber. Keep the guard alive until exit, or buffered lines are lost.
pub fn init(log_dir: &Path, verbose: u8) -> std::io::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)?;
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("cartograph")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir)
        .map_err(std::io::Error::other)?;
    let (file, guard) = tracing_appender::non_blocking(file);
    let stderr_level = match verbose {
        0 => None,
        1 => Some("info"),
        2 => Some("debug"),
        _ => Some("trace"),
    };
    let registry = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .json()
                .with_writer(file)
                .with_filter(EnvFilter::new("info")),
        )
        .with(stderr_level.map(|l| {
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(EnvFilter::new(l))
        }));
    // A second init (tests, embedded use) keeps the first subscriber.
    let _ = registry.try_init();
    Ok(guard)
}
