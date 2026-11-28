//! Orchestration: sync, rebuild, overlays, layout and freshness (D1).

pub mod aliases;
pub mod build;
pub mod config;
pub mod contracts;
pub mod corrections;
pub mod derive;
pub mod entrypoints;
pub mod events;
pub mod extract;
pub mod freshness;
pub mod http;
pub mod ifaces;
pub mod layout;
pub mod links;
pub mod owners;
mod rows;
pub mod rules;
pub mod structure;
pub mod sync;
mod tables;
pub mod timeline;

use std::path::Path;

pub use build::{BuildError, Summary, build_single, default_workers};
use cartograph_discovery as _;
use cartograph_facts::FactCache;
use cartograph_store::writer::WriterLock;

/// Builds `repo` up to `to` (default HEAD) into a new file and renames it over `index` (IDX-16).
/// The old index stays readable until the rename; a failed or killed build leaves it untouched.
pub fn rebuild_single(
    lock: &WriterLock,
    repo: &Path,
    to: Option<&str>,
    index: &Path,
    cache: &mut FactCache,
    workers: usize,
) -> Result<Summary, BuildError> {
    // Positions carry over by node key, so a rebuilt index opens with the same layout (IDX-25).
    let carried = cartograph_store::Index::open_read(index)
        .ok()
        .and_then(|ix| layout::positions_by_key(&ix.conn).ok());
    cartograph_store::migrate::rebuild(lock, index, |tmp| {
        build::build_single_with(repo, to, tmp, cache, workers, carried)
    })
}

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
