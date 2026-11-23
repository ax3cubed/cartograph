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
    refuse_synced(&dir, &env)?;
    Ok(dir)
}

/// Refuses a data folder or `index_path` inside iCloud Drive, Dropbox, OneDrive or Google Drive (B1).
/// On Windows the `OneDrive*` variables name the OneDrive roots wherever the user put them.
pub fn refuse_synced(p: &Path, env: impl Fn(&str) -> Option<PathBuf>) -> Result<(), CoreError> {
    let onedrive = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|k| env(k))
        .any(|root| p.starts_with(root));
    if onedrive || is_synced(p) {
        return Err(CoreError::SyncedDataDir {
            path: p.display().to_string(),
        });
    }
    Ok(())
}

/// The registry of workspaces and repos (C4).
pub fn registry_file(data: &Path) -> PathBuf {
    data.join("registry.toml")
}

/// Creates a folder (and its parents) readable only by the user (B1).
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// Takes one folder that may already exist to 0700 (B1): [`create_private_dir`] leaves the mode of
/// an existing folder alone. A no-op off Unix.
pub fn restrict_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Writes `bytes` to `path` atomically: a user-only temp file beside it, fsync, rename. A reader
/// sees the old file or the new one, never half of one.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Takes every file under `dir` to 0600 and every folder to 0700, without following symlinks, so a
/// file another crate created under the default umask (a SQLite `-wal`, a log) isn't left readable
/// by other users. A no-op off Unix, where the folder inherits the user profile's ACL.
// ponytail: a sweep after the fact. Its ceiling is a file created after it runs; the upgrade is each
// writer creating its files 0600 itself, as cartograph-store already does for the index.
pub fn make_private(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::ErrorKind::NotFound;
        use std::os::unix::fs::PermissionsExt;
        // Other processes add and delete files here while the sweep runs (a sync's dirty flag and
        // replay snapshot, SQLite's -wal, a temp file renamed away), so an entry that vanished
        // between the listing and the chmod is skipped, not an error.
        let meta = match std::fs::symlink_metadata(dir) {
            Err(e) if e.kind() == NotFound => return Ok(()),
            m => m?,
        };
        let set = |mode| match std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
        {
            Err(e) if e.kind() == NotFound => Ok(()),
            r => r,
        };
        if meta.is_dir() {
            set(0o700)?;
            let entries = match std::fs::read_dir(dir) {
                Err(e) if e.kind() == NotFound => return Ok(()),
                it => it?,
            };
            for entry in entries {
                match entry {
                    Err(e) if e.kind() == NotFound => {}
                    entry => make_private(&entry?.path())?,
                }
            }
        } else if meta.is_file() {
            set(0o600)?;
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Marks the data folder as excluded from Time Machine (B1): indexes are large and can be rebuilt.
/// Best effort; a failure only means the folder is backed up.
// ponytail: `tmutil addexclusion` sets the com_apple_backup_excludeItem attribute for us; writing
// the attribute directly would need an xattr dependency for one call.
pub fn exclude_from_backup(dir: &Path) {
    #[cfg(target_os = "macos")]
    // tmutil can take ~11 s, so it isn't awaited: a thread reaps it, and the exclusion still
    // lands if the CLI exits first (the child outlives us).
    if let Ok(mut child) = std::process::Command::new("/usr/bin/tmutil")
        .arg("addexclusion")
        .arg(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || child.wait());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = dir;
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
    fn a_file_that_vanishes_during_the_sweep_is_skipped() {
        // The sweep reaches an entry after another process deleted it (a sync's dirty flag): that
        // is the same as the path not existing when make_private looks at it.
        let gone = std::env::temp_dir().join(format!(
            "cartograph-gone-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        assert!(make_private(&gone).is_ok());
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
    fn each_synced_root_is_refused() {
        for p in [
            "/Users/a/Library/Mobile Documents/com~apple~CloudDocs/c",
            "/Users/a/Library/CloudStorage/GoogleDrive-a@b.c/My Drive/c",
            "/Users/a/Dropbox/c",
            "/Users/a/OneDrive - Acme/c",
            "/Users/a/Google Drive/c",
        ] {
            let got = refuse_synced(Path::new(p), |_| None);
            assert!(matches!(got, Err(CoreError::SyncedDataDir { .. })), "{p}");
        }
        let onedrive = |k: &str| (k == "OneDriveCommercial").then(|| PathBuf::from("D:/Work"));
        assert!(refuse_synced(Path::new("D:/Work/index"), onedrive).is_err());
        assert!(refuse_synced(Path::new("/Users/a/code/index"), onedrive).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn written_files_and_swept_folders_are_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let tmp = std::env::temp_dir().join(format!("cartograph-core-{}", std::process::id()));
        let dir = tmp.join("a/b");
        create_private_dir(&dir).unwrap();
        write_private(&dir.join("registry.toml"), b"x").unwrap();
        assert_eq!(mode(&dir.join("registry.toml")), 0o600);
        std::fs::write(dir.join("loose.log"), b"x").unwrap();
        std::fs::set_permissions(
            dir.join("loose.log"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        make_private(&tmp).unwrap();
        assert_eq!(mode(&dir.join("loose.log")), 0o600);
        assert_eq!(mode(&tmp.join("a")), 0o700);
        std::fs::remove_dir_all(&tmp).unwrap();
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
