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
    pub all_workspaces: bool,
    pub hook_args: Option<String>,
}

/// The registered workspace covering `dir` (IDX-11): the registry is the allow-list, so only its
/// repos are read or fetched (F8).
fn workspace(data: &Path, dir: &Path) -> Result<(String, Workspace), Report> {
    let applied = setup::locate(data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    let ws = cartograph_server::workspace(data, &applied.workspace_id).map_err(|p| Report {
        what: p.what,
        why: p.why,
        action: p.action,
        exit_code: ExitCode::Error,
    })?;
    Ok((applied.workspace_id, ws))
}

/// FE-16: the run's activity rows. A failure to write them never fails the sync.
fn record(data: &Path, id: &str) {
    cartograph_server::activity::after_cli_sync(data, id);
}

pub fn run(out: &Out, dir: &Path, f: Flags, g: &Global) -> Result<(), Report> {
    let data = data_dir()?;
    if f.all_workspaces {
        return all(out, &data, &f, g);
    }
    // A git hook (IDX-26): a file checkout or a rebase in progress starts nothing, not even the
    // lock try.
    if let Some(args) = &f.hook_args
        && crate::hooks::skip(dir, args)
    {
        out.print(
            "skipped: a file checkout or a rebase in progress",
            json!({ "background": "skipped" }),
        );
        return Ok(());
    }
    let (id, ws) = workspace(&data, dir)?;
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
        SyncOutcome::Synced(s) => {
            record(&data, &id);
            s
        }
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
    let (files_total, files_parsed, imports_total, imports_resolved) = s.coverage;
    if s.steps > 0 {
        let pct = if imports_total == 0 {
            100.0
        } else {
            imports_resolved as f64 * 100.0 / imports_total as f64
        };
        human += &format!(
            "\nfiles parsed {files_parsed} / {files_total}, imports resolved {pct:.1}% \
             (`cartograph status --unresolved` lists the rest)"
        );
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
            "coverage": { "files_total": files_total, "files_parsed": files_parsed,
                          "imports_total": imports_total, "imports_resolved": imports_resolved },
            "derived_rebuilt": s.derived.rebuilt(),
            "ms": { "total": s.total_ms, "rehydrate": s.rehydrate_ms, "walk": s.walk_ms,
                    "replay": s.replay_ms },
        }),
    );
    Ok(())
}

/// `sync --all-workspaces` (IDX-27): every registered workspace, one after another. One that fails
/// or whose writer is busy doesn't stop the rest; a busy one has its dirty flag set, so its running
/// sync does one more round.
fn all(out: &Out, data: &Path, f: &Flags, g: &Global) -> Result<(), Report> {
    let _guard = logging::init(&data.join("logs"), g.verbose).ok();
    let cache_path = paths::facts_cache(data);
    let mut cache = FactCache::open(&cache_path).map_err(|e| Report {
        what: format!("can't open the fact cache at {}", cache_path.display()),
        why: e.to_string(),
        action: format!("delete {}; it is only a cache", cache_path.display()),
        exit_code: ExitCode::Error,
    })?;
    let opts = SyncOptions {
        fetch: f.fetch,
        wait: false,
        ..SyncOptions::default()
    };
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    for id in cartograph_server::workspace_ids(data) {
        let (word, detail) = match cartograph_server::workspace(data, &id) {
            Err(p) => ("failed", p.what),
            Ok(ws) => match sync(&ws, &opts, &mut cache) {
                Ok(SyncOutcome::AlreadyRunning) => ("busy", String::new()),
                Ok(SyncOutcome::Synced(s)) => {
                    record(data, &id);
                    let mut detail = format!("{} commits", s.steps);
                    if !s.fetch_failed.is_empty() {
                        detail += &format!("; fetch failed: {}", s.fetch_failed.join(", "));
                    }
                    ("synced", detail)
                }
                Err(e) => ("failed", cartograph_core::Explain::what(&e)),
            },
        };
        lines.push(format!("{id}: {word} {detail}").trim_end().to_owned());
        rows.push(json!({ "workspace": id, "outcome": word, "detail": detail }));
    }
    out.print(&lines.join("\n"), json!({ "workspaces": rows }));
    Ok(())
}
