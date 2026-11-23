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
    // The first command to run (often `open` or `index`, before any `init`) creates the data
    // folder here. Both folders are user-only (B1), including ones an earlier version created under
    // the default umask: a folder other users can't enter keeps the log files private whatever
    // mode the appender gives them.
    crate::paths::create_private_dir(log_dir)?;
    crate::paths::restrict_dir(log_dir)?;
    if let Some(parent) = log_dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::paths::restrict_dir(parent)?;
    }
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

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_data_and_log_folders_are_user_only_even_when_they_existed() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root =
            std::env::temp_dir().join(format!("cartograph-log-{}-{nanos}", std::process::id()));
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        // As an earlier `cartograph open` under umask 022 left it.
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        let guard = super::init(&data.join("logs"), 0).unwrap();
        for dir in [data.clone(), data.join("logs")] {
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
        }
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }
}
