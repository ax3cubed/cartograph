//! Where Cartograph keeps everything (B1): one folder, `~/.cartograph`, on every platform.
//!
//! ```text
//! ~/.cartograph/registry.toml, config.toml
//! ~/.cartograph/cache/facts.db, cache/summaries.db
//! ~/.cartograph/workspaces/<id>/index.cartograph, ask.db, logs/, reports/, index.lock
//! ```

use std::path::{Path, PathBuf};

use crate::error::CoreError;

/// Resolves the data folder from the environment. `CARTOGRAPH_HOME` moves the whole folder; on Linux
/// an explicitly set `XDG_DATA_HOME` is honoured; otherwise `~/.cartograph` (`%USERPROFILE%` on Windows).
pub fn data_dir() -> Result<PathBuf, CoreError> {
    data_dir_from(|k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}

/// [`data_dir`] with the environment passed in, for tests.
pub fn data_dir_from(env: impl Fn(&str) -> Option<PathBuf>) -> Result<PathBuf, CoreError> {
    let dir = if let Some(home) = env("CARTOGRAPH_HOME") {
        home
    } else if let Some(xdg) = env("XDG_DATA_HOME").filter(|_| cfg!(target_os = "linux")) {
        xdg.join("cartograph")
    } else {
        env("HOME")
            .or_else(|| env("USERPROFILE"))
            .ok_or(CoreError::NoHome)?
            .join(".cartograph")
    };
    if is_synced(&dir) {
        return Err(CoreError::SyncedDataDir {
            path: dir.display().to_string(),
        });
    }
    Ok(dir)
}

/// The folder of one workspace's index, Ask history, logs and lock.
pub fn workspace_dir(data: &Path, workspace_id: &str) -> PathBuf {
    data.join("workspaces").join(workspace_id)
}

/// The shared parse cache (ruling R8).
pub fn facts_cache(data: &Path) -> PathBuf {
    data.join("cache").join("facts.db")
}

/// True for paths inside iCloud Drive, Dropbox, OneDrive or Google Drive, which B1 refuses.
fn is_synced(p: &Path) -> bool {
    p.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "Mobile Documents"
            || s == "Dropbox"
            || s.starts_with("OneDrive")
            || s.starts_with("Google Drive")
            || s == "CloudStorage"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<PathBuf> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| PathBuf::from(v))
        }
    }

    #[test]
    fn home_gives_dot_cartograph() {
        assert_eq!(
            data_dir_from(env(&[("HOME", "/h")])).unwrap(),
            PathBuf::from("/h/.cartograph")
        );
    }

    #[test]
    fn cartograph_home_wins() {
        let got = data_dir_from(env(&[("HOME", "/h"), ("CARTOGRAPH_HOME", "/data/c")])).unwrap();
        assert_eq!(got, PathBuf::from("/data/c"));
    }

    #[test]
    fn synced_folders_are_refused() {
        let got = data_dir_from(env(&[(
            "CARTOGRAPH_HOME",
            "/Users/a/Library/CloudStorage/Dropbox/c",
        )]));
        assert!(matches!(got, Err(CoreError::SyncedDataDir { .. })));
    }
}
