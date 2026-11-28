//! The server's calls into the indexer, discovery and the registry. Every one is the same function
//! the CLI uses (A1): the server holds no indexing logic of its own.

use std::path::{Path, PathBuf};

use cartograph_api::types::{Freshness, IndexState, JobKind, Problem, Repo, RepoBehind};
use cartograph_core::RepoId;
use cartograph_core::{Explain, paths};
use cartograph_discovery::Registry;
use cartograph_discovery::manifest;
use cartograph_facts::FactCache;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{QueryError, Snapshot};

fn problem(e: &dyn Explain) -> Problem {
    Problem {
        what: e.what(),
        why: e.why(),
        action: e.action(),
    }
}

/// Registered workspace ids, sorted. The registry is the allow-list: nothing outside it is read
/// or synced (F8).
pub fn workspace_ids(data: &Path) -> Vec<String> {
    let mut ids: Vec<String> = Registry::load(data)
        .map(|(r, _)| r.workspaces.into_iter().map(|w| w.id).collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// The workspace as sync sees it: its folder and its repos, from the registry and the manifest.
pub fn workspace(data: &Path, ws: &str) -> Result<Workspace, Problem> {
    workspace_checked(data, ws).map(|(w, _)| w)
}

/// [`workspace`], and the names of the manifest's repos that it leaves out.
///
/// The registry is the allow-list (F8): a committed manifest can name any folder (`../..`, an
/// absolute path), so only a repo whose canonical path is registered for this workspace is synced
/// or fetched. The rest, including a repo not cloned on this machine, are reported as not added
/// until `cartograph init` (or the app's folder picker) registers them.
pub fn workspace_checked(data: &Path, ws: &str) -> Result<(Workspace, Vec<String>), Problem> {
    let (reg, _) = Registry::load(data).map_err(|e| problem(&e))?;
    let entry = reg
        .workspaces
        .iter()
        .find(|w| w.id == ws)
        .ok_or_else(|| Problem {
            what: "no such workspace".into(),
            why: "the registry has no workspace with this id".into(),
            action: "add it from the app or with cartograph init".into(),
        })?;
    let parsed = manifest::read(&entry.manifest).map_err(|e| problem(&e))?;
    let base = entry.manifest.parent().unwrap_or(Path::new("/"));
    let mut repos = Vec::new();
    let mut not_added = Vec::new();
    for r in &parsed.manifest.repos {
        let path = manifest::repo_dir(base, r);
        let registered = std::fs::canonicalize(&path)
            .is_ok_and(|c| reg.repos.iter().any(|e| e.workspace == ws && e.path == c));
        if !registered {
            not_added.push(r.name.clone());
            continue;
        }
        repos.push(RepoSpec {
            name: r.name.clone(),
            path,
            branch: r.default_branch.clone(),
            scopes: r.scopes.clone(),
        });
    }
    Ok((
        Workspace {
            dir: paths::workspace_dir(data, ws),
            repos,
            exclude: parsed.manifest.options.exclude.clone(),
            config_hash: Some(manifest::config_hash(&parsed.manifest)),
            services: parsed
                .manifest
                .services
                .iter()
                .map(|s| cartograph_index::structure::ServiceSpec {
                    name: s.name.clone(),
                    repo: s.repo.clone(),
                    path: s.path.clone(),
                })
                .collect(),
            contracts: parsed
                .manifest
                .contracts
                .iter()
                .map(cartograph_index::contracts::ContractSpec::from)
                .collect(),
            environments: parsed
                .manifest
                .environments
                .iter()
                .map(|(name, e)| cartograph_index::config::EnvSpec {
                    name: name.clone(),
                    sources: e.sources.clone(),
                    set: e.set.clone(),
                })
                .collect(),
            default_environment: parsed.manifest.default_environment.clone(),
            overrides: Some(base.join(cartograph_discovery::overrides::FILE_NAME)),
            subs: subprocess_adapters(&reg, ws, base, &parsed.manifest.adapters),
        },
        not_added,
    ))
}

/// The manifest's subprocess adapters (XS-24): only those this user approved, with this exact hash,
/// can run. A sync, MCP or the background refresh only reads the approvals; none of them gives one.
pub fn subprocess_adapters(
    reg: &Registry,
    ws: &str,
    base: &Path,
    folders: &[String],
) -> cartograph_facts::adapters::subprocess::SubSet {
    use cartograph_facts::adapters::subprocess::{Declared, SubSet};
    SubSet::new(
        folders
            .iter()
            .map(|f| Declared::load(base, f).map_err(|e| format!("{f}: {e}")))
            .collect(),
        |d| reg.adapter_allowed(ws, &d.name, &d.hash),
    )
}

/// Registers the manifest repos of `ws` that aren't registered yet, when the user chose them in
/// the app: a repo the app's settings name is a trust act, as `cartograph init` is. Only a git work
/// tree inside `home` (the folder picker's boundary) and not registered elsewhere is added; any
/// other stays out of sync and is reported as not added. Returns the names added.
pub fn register_chosen(data: &Path, ws: &str, home: Option<&Path>) -> Vec<String> {
    let Some(home) = home.and_then(|h| std::fs::canonicalize(h).ok()) else {
        return Vec::new();
    };
    let Ok((reg, _)) = Registry::load(data) else {
        return Vec::new();
    };
    let Some(entry) = reg.workspaces.iter().find(|w| w.id == ws) else {
        return Vec::new();
    };
    let Ok(parsed) = manifest::read(&entry.manifest) else {
        return Vec::new();
    };
    let base = entry.manifest.parent().unwrap_or(Path::new("/"));
    let chosen: Vec<(String, PathBuf)> = parsed
        .manifest
        .repos
        .iter()
        .filter_map(|r| {
            let c = std::fs::canonicalize(manifest::repo_dir(base, r)).ok()?;
            let ok = c.starts_with(&home)
                && c.join(".git").exists()
                && !reg.repos.iter().any(|e| e.path == c);
            ok.then(|| (r.name.clone(), c))
        })
        .collect();
    if chosen.is_empty() {
        return Vec::new();
    }
    let added = Registry::update(data, |reg| {
        let mut added = Vec::new();
        for (name, path) in &chosen {
            if reg.repos.iter().any(|e| &e.path == path) {
                continue;
            }
            let (remote, root_commit) = cartograph_discovery::registry::repo_identity(path);
            reg.repos.push(cartograph_discovery::registry::RepoEntry {
                path: path.clone(),
                workspace: ws.to_owned(),
                remote,
                root_commit,
            });
            added.push(name.clone());
        }
        reg.repos.sort_by(|a, b| a.path.cmp(&b.path));
        added
    });
    added.unwrap_or_default()
}

/// What a read says about a manifest repo left out of sync.
pub fn not_added_problem(names: &[String]) -> Problem {
    Problem {
        what: format!("not added: {}", names.join(", ")),
        why: "cartograph.yaml names these repos, but this machine hasn't registered them, so they \
              aren't synced"
            .into(),
        action: "run `cartograph init` in the workspace folder".into(),
    }
}

/// The folder holding the workspace's `cartograph.yaml`.
pub fn manifest_path(data: &Path, ws: &str) -> Option<PathBuf> {
    let (reg, _) = Registry::load(data).ok()?;
    reg.workspaces
        .into_iter()
        .find(|w| w.id == ws)
        .map(|w| w.manifest)
}

/// How current the index is (D1): the stored state from the snapshot, with each repo's HEAD
/// compared to its indexed commit by IDX-15's check (ref reads only, memoised for 1 s). A
/// workspace that isn't in the registry (a single-repo `cartograph index`) has the stored state.
pub fn freshness(data: &Path, ws: &str, snap: &Snapshot<'_>) -> Result<Freshness, QueryError> {
    use cartograph_index::freshness::State as S;
    let mut f = snap.freshness()?;
    let Ok((w, not_added)) = workspace_checked(data, ws) else {
        return Ok(f);
    };
    if !not_added.is_empty() && f.error.is_none() {
        f.error = Some(not_added_problem(&not_added));
    }
    let r = cartograph_index::freshness::freshness(&w);
    f.state = match r.state {
        S::None => IndexState::None,
        S::Current => IndexState::Current,
        S::Behind => IndexState::Behind,
        S::Syncing => IndexState::Syncing,
        S::Rewritten => IndexState::Rewritten,
        S::NeedsMigrate => IndexState::NeedsMigrate,
        S::Failed => IndexState::Failed,
    };
    // A sync in any process (the CLI, a hook) says how far it is through its writer's file.
    if f.state == IndexState::Syncing && f.progress.is_none() {
        f.progress = read_progress(&w.dir);
    }
    // A failed index says why and what to run, instead of a bare "Index failed".
    if f.state == IndexState::Failed && f.error.is_none() {
        f.error = Some(Problem {
            what: "the index isn't complete".into(),
            why: snap.last_error().ok().flatten().unwrap_or_else(|| {
                "a sync stopped part way through, or another version wrote the file".into()
            }),
            action: "run `cartograph sync`, which resumes from the last batch".into(),
        });
    }
    f.repos = r
        .repos
        .iter()
        .map(|x| RepoBehind {
            repo: RepoId(x.name.clone()),
            name: x.name.clone(),
            behind: x.behind,
            behind_capped: x.behind_capped,
        })
        .collect();
    f.behind = f.repos.iter().map(|x| x.behind).sum();
    Ok(f)
}

/// How far the running sync is, from the `progress.json` its writer keeps: `None` before its first
/// step or when unreadable.
fn read_progress(dir: &Path) -> Option<f32> {
    let bytes = std::fs::read(dir.join(cartograph_store::writer::PROGRESS_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let (done, total) = (v["done"].as_u64()?, v["total"].as_u64()?);
    (total > 0).then(|| done.min(total) as f32 / total as f32)
}

/// `Repo[]` with each repo's HEAD and commits behind from the same check as [`freshness`].
pub fn repos(data: &Path, ws: &str, snap: &Snapshot<'_>) -> Result<Vec<Repo>, QueryError> {
    let mut repos = snap.repos()?;
    let Ok(w) = workspace(data, ws) else {
        return Ok(repos);
    };
    let r = cartograph_index::freshness::freshness(&w);
    for repo in &mut repos {
        if let Some(f) = r.repos.iter().find(|f| f.name == repo.name) {
            repo.head = f.head.map(|o| o.to_string()).unwrap_or_default();
            repo.indexed_commit = f.indexed.map(|o| o.to_string());
            repo.behind = f.behind;
            repo.behind_capped = f.behind_capped;
            repo.dirty = w
                .repos
                .iter()
                .find(|s| s.name == repo.name)
                .and_then(|s| cartograph_index::freshness::uncommitted(&s.path));
        }
    }
    Ok(repos)
}

/// `auto_sync` from the machine settings (config.toml). Background unless set to never; a file
/// that doesn't parse starts nothing, since it may be a mistyped "never".
pub fn auto_sync(data: &Path) -> bool {
    match cartograph_discovery::config::Config::load(data) {
        Ok(c) => c.auto_sync == cartograph_discovery::config::AutoSync::Background,
        Err(e) => {
            tracing::warn!(error = %e.what(), why = %e.why(), "auto_sync is off until config.toml parses");
            false
        }
    }
}

/// Runs one sync of `ws` and returns the index generation it ended at. A rebuild is a sync into
/// a fresh file renamed into place, so the previous index stays readable until it's complete.
// `progress` gets the start, each step of each round (steps written of the round's total) and the
// end. A cancel stops the writer after its current batch (E5): a sync keeps the steps it wrote, an
// earlier but complete index; a rebuild is thrown away.
pub fn run(
    data: &Path,
    ws: &str,
    kind: JobKind,
    progress: impl FnMut(&str, u64, u64),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<u64, Problem> {
    run_with(data, ws, kind, progress, cancel, SyncOptions::default())
}

/// [`run`] that waits for a writer holding the lock, then runs its own round: MCP `sync` promises
/// a result, and a lock held for a moment (a freshness probe takes it shared) must not end the
/// job with nothing synced (SRV-16).
pub fn run_waiting(
    data: &Path,
    ws: &str,
    progress: impl FnMut(&str, u64, u64),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<u64, Problem> {
    let opts = SyncOptions {
        wait: true,
        ..SyncOptions::default()
    };
    run_with(data, ws, JobKind::Sync, progress, cancel, opts)
}

fn run_with(
    data: &Path,
    ws: &str,
    kind: JobKind,
    mut progress: impl FnMut(&str, u64, u64),
    cancel: &std::sync::atomic::AtomicBool,
    opts: SyncOptions,
) -> Result<u64, Problem> {
    let mut w = workspace(data, ws)?;
    let index = w.index_path();
    let rebuild = matches!(kind, JobKind::Rebuild) && index.exists();
    if rebuild {
        w.dir = w.dir.join("rebuild");
        let _ = std::fs::remove_dir_all(&w.dir);
    }
    progress("sync", 0, 1);
    let cache_path = paths::facts_cache(data);
    let mut cache = FactCache::open(&cache_path).map_err(|e| Problem {
        what: "can't open the fact cache".into(),
        why: e.to_string(),
        action: "delete the cache folder in the data folder; it is only a cache".into(),
    })?;
    // Every step reports, between the start and the end; the live channel throttles.
    let outcome =
        cartograph_index::sync::sync_with_progress(&w, &opts, &mut cache, &mut |done, total| {
            progress("sync", done, total.max(1))
        })
        .map_err(|e| problem(&e))?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        if rebuild {
            let _ = std::fs::remove_dir_all(&w.dir);
        }
        return Err(Problem {
            what: "cancelled".into(),
            why: "the job was stopped".into(),
            action: "start it again".into(),
        });
    }
    if rebuild {
        let installed = install_rebuilt(&paths::workspace_dir(data, ws), &w.index_path(), &index);
        let _ = std::fs::remove_dir_all(&w.dir);
        installed?;
    }
    // A config saved while this job ran (PUT /config only asks the running job for one more round)
    // isn't in `w`, read when the job started, and freshness compares heads only, so nothing would
    // sync it. Read the workspace again; if its config changed, sync once more with it.
    if let Ok(again) = workspace(data, ws)
        && again.config_hash != w.config_hash
    {
        cartograph_index::sync::sync_with_progress(
            &again,
            &SyncOptions::default(),
            &mut cache,
            &mut |done, total| progress("sync", done, total.max(1)),
        )
        .map_err(|e| problem(&e))?;
    }
    if let SyncOutcome::AlreadyRunning = outcome {
        tracing::info!(ws, "a sync was already running; it runs one more round");
    }
    progress("sync", 1, 1);
    Ok(generation(&index))
}

/// IDX-16: the rebuilt file goes in under the workspace's own writer lock and through the store's
/// install (the old file checkpointed, its -wal and -shm removed), never a bare rename. SQLite
/// finds a log by name, so the old file's -wal would be read as the new file's, and a sync holding
/// the lock would keep writing a file that is no longer the index.
fn install_rebuilt(dir: &Path, built: &Path, index: &Path) -> Result<(), Problem> {
    let failed = |why: String| Problem {
        what: "can't move the rebuilt index into place".into(),
        why,
        action: "check the data folder's permissions and free space".into(),
    };
    let Some(_lock) = cartograph_store::writer::WriterLock::try_acquire(dir)
        .map_err(|e| failed(e.to_string()))?
    else {
        return Err(Problem {
            what: "a sync is writing the index".into(),
            why: "the rebuilt index can't replace it while another writer holds it".into(),
            action: "start the rebuild again once that sync is done".into(),
        });
    };
    std::fs::rename(built, cartograph_store::migrate::new_path(index))
        .map_err(|e| failed(e.to_string()))?;
    cartograph_store::migrate::install(index).map_err(|e| failed(e.to_string()))
}

fn generation(index: &Path) -> u64 {
    cartograph_query::Store::open_path(index)
        .and_then(|s| s.snapshot(Default::default()).map(|snap| snap.generation))
        .unwrap_or(0)
}

/// D2: asks the running writer for one more round.
pub fn request_round(data: &Path, ws: &str) {
    let _ = cartograph_store::writer::mark_dirty(&paths::workspace_dir(data, ws));
}

/// E5: asks the running writer to stop after its current batch.
pub fn request_cancel(data: &Path, ws: &str) {
    let dir = paths::workspace_dir(data, ws);
    let _ = cartograph_store::writer::request_cancel(&dir);
    // A rebuild writes in <ws>/rebuild under its own lock, which looks for its own cancel file.
    let rebuild = dir.join("rebuild");
    if rebuild.is_dir() {
        let _ = cartograph_store::writer::request_cancel(&rebuild);
    }
}

/// Takes back a cancel request the running writer hasn't acted on yet (the job was asked for again).
pub fn withdraw_cancel(data: &Path, ws: &str) {
    let _ = std::fs::remove_file(
        paths::workspace_dir(data, ws).join(cartograph_store::writer::CANCEL_FILE),
    );
}

/// Removes a workspace from the registry and deletes its data folder (a cancelled first index).
/// The manifest in the user's folder is theirs and stays.
pub fn forget(data: &Path, ws: &str) {
    let _ = Registry::update(data, |r| {
        r.workspaces.retain(|w| w.id != ws);
        r.repos.retain(|x| x.workspace != ws);
    });
    if cartograph_core::ids::is_workspace_id(ws) {
        let _ = std::fs::remove_dir_all(paths::workspace_dir(data, ws));
    }
}

#[cfg(test)]
mod tests {
    use cartograph_discovery::Registry;
    use cartograph_discovery::registry::{RepoEntry, WorkspaceEntry};

    /// A home with a workspace `ws/` whose manifest names a registered repo, an unregistered one
    /// inside home, one outside home and one not cloned. Returns (data, home, outside).
    fn setup(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::fs::canonicalize(root).unwrap();
        let (home, data) = (root.join("home"), root.join("data"));
        for d in ["home/ws/a/.git", "home/ws/b/.git", "out/.git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let manifest = home.join("ws/cartograph.yaml");
        std::fs::write(
            &manifest,
            "version: 1\nworkspace:\n  name: w\nrepos:\n- name: a\n  path: a\n- name: b\n  path: b\n\
             - name: out\n  path: ../../out\n- name: gone\n  path: gone\n",
        )
        .unwrap();
        Registry::update(&data, |r| {
            r.register(
                WorkspaceEntry {
                    id: "0123abcd".into(),
                    name: "w".into(),
                    manifest: manifest.clone(),
                },
                vec![RepoEntry {
                    path: home.join("ws/a"),
                    workspace: "0123abcd".into(),
                    remote: None,
                    root_commit: None,
                }],
            )
        })
        .unwrap();
        (data, home)
    }

    #[test]
    fn only_registered_repos_are_synced_and_the_rest_are_reported() {
        let d = tempfile::tempdir().unwrap();
        let (data, _) = setup(d.path());
        let (w, not_added) = super::workspace_checked(&data, "0123abcd").unwrap();
        let names: Vec<&str> = w.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a"]);
        assert_eq!(not_added, ["b", "out", "gone"]);
        assert!(
            super::not_added_problem(&not_added)
                .action
                .contains("cartograph init")
        );
    }

    #[test]
    fn a_repo_chosen_in_the_app_is_registered_only_inside_home() {
        let d = tempfile::tempdir().unwrap();
        let (data, home) = setup(d.path());
        assert_eq!(
            super::register_chosen(&data, "0123abcd", Some(&home)),
            ["b"]
        );
        let (w, not_added) = super::workspace_checked(&data, "0123abcd").unwrap();
        assert_eq!(w.repos.len(), 2);
        assert_eq!(not_added, ["out", "gone"]);
        assert!(super::register_chosen(&data, "0123abcd", Some(&home)).is_empty());
        assert!(super::register_chosen(&data, "0123abcd", None).is_empty());
    }

    #[test]
    fn auto_sync_follows_config_toml_and_a_broken_file_starts_nothing() {
        let d = tempfile::tempdir().unwrap();
        assert!(super::auto_sync(d.path()), "background by default");
        let file = d.path().join("config.toml");
        std::fs::write(&file, "auto_sync = \"never\"\n").unwrap();
        assert!(!super::auto_sync(d.path()));
        std::fs::write(&file, "auto_sync = \"background\"\n").unwrap();
        assert!(super::auto_sync(d.path()));
        std::fs::write(&file, "auto_sync = never\n").unwrap();
        assert!(!super::auto_sync(d.path()));
    }
}

#[cfg(test)]
mod job_tests {
    use cartograph_api::types::JobKind;

    #[test]
    fn a_sync_in_another_process_reports_its_progress() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join(cartograph_store::writer::PROGRESS_FILE);
        assert_eq!(super::read_progress(d.path()), None, "no sync, no progress");
        std::fs::write(
            &file,
            r#"{"pid":1,"phase":"replay","done":25,"total":100,"repo":null}"#,
        )
        .unwrap();
        assert_eq!(super::read_progress(d.path()), Some(0.25));
        // Before its first step a run writes total 0: unknown, not 100%.
        std::fs::write(
            &file,
            r#"{"pid":1,"phase":"walk","done":0,"total":0,"repo":null}"#,
        )
        .unwrap();
        assert_eq!(super::read_progress(d.path()), None);
    }

    #[test]
    fn a_config_saved_while_a_job_runs_is_synced_by_that_job() {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let mut f = cartograph_git::fixture::Fixture::init(&root.join("repo"));
        f.write("src/a.ts", "export const a = 1;\n");
        f.write("gen/b.ts", "export const b = 1;\n");
        f.commit("one");
        let data = root.join("data");
        let plan = cartograph_discovery::setup::plan(&f.dir, None).unwrap();
        let applied = cartograph_discovery::setup::apply(&plan, &data).unwrap();
        let manifest = plan.manifest_path.clone();
        let mut saved = false;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        super::run(
            &data,
            &applied.workspace_id,
            JobKind::Sync,
            |_, done, _| {
                // PUT /config lands while the job's first round runs.
                if !saved && done > 0 {
                    saved = true;
                    let mut m = cartograph_discovery::manifest::read(&manifest)
                        .unwrap()
                        .manifest;
                    m.options.exclude.push("gen/**".into());
                    std::fs::write(&manifest, cartograph_discovery::manifest::to_yaml(&m)).unwrap();
                }
            },
            &cancel,
        )
        .unwrap();
        assert!(saved);
        let ix = cartograph_store::Index::open_read(&applied.index).unwrap();
        let live: Vec<String> = ix
            .conn
            .prepare("SELECT path FROM file_path WHERE to_seq IS NULL")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(live, ["src/a.ts"], "the exclude saved mid-job is applied");
    }
}
