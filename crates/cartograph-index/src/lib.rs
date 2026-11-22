//! Orchestration: sync, rebuild, overlays, layout and freshness (D1).

pub mod build;
pub mod extract;

use std::path::Path;

pub use build::{BuildError, Summary, build_single, default_workers};
use cartograph_discovery as _;

/// A workspace id for a repository folder, until the registry (IDX-11) assigns them: the folder's
/// name and a hash of its canonical path.
pub fn workspace_id(repo: &Path) -> String {
    let canon = repo.canonicalize().unwrap_or_else(|_| repo.to_owned());
    let name: String = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // FNV-1a: stable across runs and platforms, unlike std's hasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canon.to_string_lossy().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{name}-{:08x}", h as u32)
}

#[cfg(test)]
mod tests;
