//! `~/.cartograph/config.toml`: this machine's settings (B1). Only the user writes it; nothing in a
//! repo can set these. A missing file is the defaults; keys this build doesn't know are ignored, so
//! a newer build's settings don't break an older one.
//!
//! ```toml
//! auto_sync = "never"   # or "background" (the default): reads start a sync when behind (D1)
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::DiscoveryError;

/// Whether a read that finds the index behind starts a sync (D1). Explicit syncs (the CLI, the
/// app's Sync button, the git hooks) run either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoSync {
    #[default]
    Background,
    Never,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub auto_sync: AutoSync,
}

pub fn config_file(data: &Path) -> PathBuf {
    data.join("config.toml")
}

impl Config {
    pub fn load(data: &Path) -> Result<Self, DiscoveryError> {
        let path = config_file(data);
        let err = |why: String| DiscoveryError::Config {
            path: path.display().to_string(),
            why,
        };
        match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).map_err(|e| err(e.message().to_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(err(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_values_and_errors() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(d.path()).unwrap().auto_sync,
            AutoSync::Background
        );
        std::fs::write(
            config_file(d.path()),
            "auto_sync = \"never\"\n[providers]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(Config::load(d.path()).unwrap().auto_sync, AutoSync::Never);
        std::fs::write(config_file(d.path()), "auto_sync = \"sometimes\"\n").unwrap();
        let e = Config::load(d.path()).unwrap_err();
        assert!(matches!(e, DiscoveryError::Config { .. }), "{e:?}");
    }
}
