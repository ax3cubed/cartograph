//! `cartograph sync` (IDX-14): index the commits added since the last run, in the workspace that
//! `[dir]` belongs to. A sync that finds another running sets the dirty flag and exits 0 (D2).
//! `--background` asks for a sync and returns at once (IDX-15): what the git hooks run.

use std::path::Path;

use cartograph_core::{ExitCode, Report, logging, paths};
use cartograph_discovery::setup;
use cartograph_facts::FactCache;
use cartograph_index::freshness::{AutoSync, SyncRequest, request_sync};
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use serde_json::json;

use crate::output::Out;
use crate::{Global, data_dir};

pub struct Flags {
    pub fetch: bool,
    pub wait: bool,
    pub background: bool,
}

/// The registered workspace covering `dir` (IDX-11): the registry is the allow-list, so only its
/// repos are read or fetched (F8).
fn workspace(data: &Path, dir: &Path) -> Result<Workspace, Report> {
    let applied = setup::locate(data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    cartograph_server::workspace(data, &applied.workspace_id).map_err(|p| Report {
        what: p.what,
        why: p.why,
        action: p.action,
        exit_code: ExitCode::Error,
    })
}

pub fn run(out: &Out, dir: &Path, f: Flags, g: &Global) -> Result<(), Report> {
    let data = data_dir()?;
    let ws = workspace(&data, dir)?;
    let explain = |e: &dyn cartograph_core::Explain| Report::from_error(e);
    if f.background {
        // An explicit request (a hook): auto_sync = "never" governs only the syncs reads start.
        let r = request_sync(&ws, AutoSync::Background, dir).map_err(|e| explain(&e))?;
        let (word, human) = match r {
            SyncRequest::Spawned => ("started", "sync started in the background"),
            SyncRequest::AlreadyRunning => (
                "already_running",
                "a sync is running; it will do one more round",
            ),
            SyncRequest::RateLimited => ("rate_limited", "a sync was started under 2 s ago"),
            SyncRequest::Disabled => ("disabled", "background sync is off"),
        };
        out.print(human, json!({ "background": word }));
        return Ok(());
    }
    let _guard = logging::init(&ws.dir.join("logs"), g.verbose).ok();
    let cache_path = paths::facts_cache(&data);
    let mut cache = FactCache::open(&cache_path).map_err(|e| Report {
        what: format!("can't open the fact cache at {}", cache_path.display()),
        why: e.to_string(),
        action: format!("delete {}; it is only a cache", cache_path.display()),
        exit_code: ExitCode::Error,
    })?;
    let opts = SyncOptions {
        fetch: f.fetch,
        wait: f.wait,
        ..SyncOptions::default()
    };
    let s = match sync(&ws, &opts, &mut cache).map_err(|e| explain(&e))? {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => {
            out.print(
                "a sync is already running; it will include this request",
                json!({ "outcome": "already_running" }),
            );
            return Ok(());
        }
    };
    let mut human = if s.steps == 0 {
        "the index is current".to_owned()
    } else {
        format!("indexed {} commits in {} ms", s.steps, s.total_ms)
    };
    for (what, names) in [
        ("added", &s.added),
        ("removed", &s.removed),
        ("rewritten, replayed from the merge base", &s.rewritten),
        ("fetch failed", &s.fetch_failed),
        (
            "package name declared by more than one repo",
            &s.duplicate_packages,
        ),
    ] {
        if !names.is_empty() {
            human += &format!("\n{what}: {}", names.join(", "));
        }
    }
    if s.dirty_left {
        human += "\nmore commits arrived than one run takes; the next sync continues";
    }
    out.print(
        &human,
        json!({
            "outcome": "synced",
            "steps": s.steps,
            "rounds": s.rounds,
            "watermark_seq": s.watermark,
            "replayed_from": s.replayed_from,
            "added": s.added,
            "removed": s.removed,
            "rewritten": s.rewritten,
            "duplicate_packages": s.duplicate_packages,
            "fetch_failed": s.fetch_failed,
            "dirty_left": s.dirty_left,
            "ms": { "total": s.total_ms, "rehydrate": s.rehydrate_ms, "walk": s.walk_ms,
                    "replay": s.replay_ms },
        }),
    );
    Ok(())
}
