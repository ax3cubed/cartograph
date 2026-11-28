//! Incremental sync (IDX-14) of a workspace's repositories onto one merged timeline (IDX-12).
//!
//! A sync is the only way an index is written for a workspace: the first one starts from an empty
//! file, so "incremental equals full" holds by construction and is checked by a property test over
//! splits of a history into runs.
//!
//! A round:
//! 1. reads the indexed steps (`commits`) and each repository's tip;
//! 2. walks each repository from its last indexed commit to its tip (from the merge base if the
//!    history was rewritten);
//! 3. finds the floor: the last indexed step that sorts before every new step. Normally that is the
//!    watermark. A repository fetched late, a folder added to the workspace or a rewrite puts new
//!    steps before it: then every step after the floor is removed and walked again, so the merged
//!    order is exactly the one a full build gives (the partial replay of IDX-14, without IDX-19);
//! 4. rehydrates the replay at the floor from the index: the tree from open `file_version` rows,
//!    the facts of those blobs from `fact`, and the links from the last round's replay snapshot
//!    (`replay.snapshot`, taken at the watermark) or, when there is none that matches, from one
//!    replay step over that tree, which resolves every reference (seconds on remotion). Links it
//!    opens are the open `interval` rows already written, and when one closes its row is updated;
//! 5. replays the merged new steps and writes them in one transaction.
//!
//! A folder removed from the workspace gets one synthetic step that deletes its live files, so
//! its open intervals close and its history stays.
//!
//! One writer at a time (B2, D2): a second sync finds the lock held, leaves a `dirty` flag and
//! returns [`SyncOutcome::AlreadyRunning`]; the running one sees the flag and runs one more round.
//!
//! Writes go through IDX-13's batches: every batch also writes the intervals that opened in it,
//! `repo.indexed_commit` and `meta.watermark_seq`, so a sync killed part way leaves an index the
//! next sync continues from.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cartograph_core::Explain;
use cartograph_facts::cache::create_private_dir;
use cartograph_facts::replay::Change;
use cartograph_facts::{FactCache, Replay, SNAPSHOT_VERSION};
use cartograph_git::{CommitStep, GitError, ObjectId, Repo};
use cartograph_parse::{BlobFacts, extractor_for, tracked};
use cartograph_store::hash::store_fact_hash;
use cartograph_store::rusqlite::{self, OptionalExtension, params};
use cartograph_store::writer::{Batches, DIRTY_FILE, Progress, WriterLock, mark_dirty, recover};
use cartograph_store::{Index, PendingCheck, StoreError};

use crate::derive::{DerivedStats, Inputs, RepoInput, coverage, refresh};
use crate::extract::{ExtractStats, Facts, extract};
use crate::ifaces;
use crate::rows::Rows;
use crate::structure::ServiceSpec;
use crate::timeline::{RepoSpec, duplicate_packages, merge, namespaced, valid_repo_name};

/// At most this many extra rounds per process for the dirty flag (D2), so a commit loop can't pin
/// a sync forever (proposed).
pub const MAX_EXTRA_ROUNDS: usize = 5;

/// A workspace as sync sees it: its data folder (`<data>/workspaces/<id>`) and its repositories.
///
/// ponytail: built by the caller until the registry (IDX-11) and discovery (IDX-08) land; then
/// `Registry::workspace_for(path)` returns this.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    /// `<data>/workspaces/<id>`: its last component is the workspace id, written to
    /// `meta.workspace_id`.
    pub dir: PathBuf,
    pub repos: Vec<RepoSpec>,
    /// The manifest's `options.exclude` (the workspace's `files.exclude`), repository-relative.
    pub exclude: Vec<String>,
    /// `manifest::config_hash`, stored as `meta.config_hash` for the fact hash (IDX-17).
    pub config_hash: Option<String>,
    /// The manifest's `services:` entries (C6's first rule, IDX-20).
    pub services: Vec<ServiceSpec>,
    /// The manifest's `contracts:` entries (XS-08).
    pub contracts: Vec<crate::contracts::ContractSpec>,
    /// The manifest's `environments:` (XS-09).
    pub environments: Vec<crate::config::EnvSpec>,
    /// The manifest's `default_environment`.
    pub default_environment: Option<String>,
    /// `cartograph.overrides.yaml` beside the manifest (XS-12), when there is a manifest.
    pub overrides: Option<PathBuf>,
    /// The subprocess adapters the user approved, and the declared ones that won't run (XS-24).
    pub subs: cartograph_facts::adapters::subprocess::SubSet,
}

impl Workspace {
    /// What decides which paths a walk keeps: the excludes on the first line, then one line per
    /// repository (name, branch, scopes).
    fn walk_config(&self) -> String {
        let mut ex = self.exclude.clone();
        ex.sort();
        // `deploy` marks the compose and k8s files the walk tracks (XS-02): an index from before
        // them has none of their history, so it is walked again. `config` marks the value-source
        // files (XS-09), `contracts` the spec files (XS-08) and the manifest's globs for more.
        let mut globs: Vec<&str> = self.contracts.iter().map(|c| c.path.as_str()).collect();
        globs.sort_unstable();
        let mut lines = vec![format!(
            "{}\0deploy\0config\0contracts\0{}",
            ex.join("\0"),
            globs.join("\0")
        )];
        for r in &self.repos {
            let mut s = r.scopes.clone();
            s.sort();
            let branch = r.branch.as_deref().unwrap_or("");
            lines.push(format!("{}\0{branch}\0{}", r.name, s.join("\0")));
        }
        lines[1..].sort();
        lines.join("\n")
    }
}

/// Whether the excludes, or the branch or scopes of a repository indexed before, changed. Adding
/// or removing a repository doesn't count: the merge rule and the removal step handle those.
fn walk_config_changed(stored: &str, now: &str) -> bool {
    let (mut a, mut b) = (stored.split('\n'), now.split('\n'));
    if a.next() != b.next() {
        return true;
    }
    let before: HashMap<&str, &str> = a.map(|l| (l.split('\0').next().unwrap_or(""), l)).collect();
    b.any(|l| {
        before
            .get(l.split('\0').next().unwrap_or(""))
            .is_some_and(|old| *old != l)
    })
}

impl Workspace {
    pub fn index_path(&self) -> PathBuf {
        self.dir.join("index.cartograph")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(cartograph_store::writer::LOCK_FILE)
    }
    /// The D2 flag: "a sync was requested while one was running".
    pub fn dirty_path(&self) -> PathBuf {
        self.dir.join(DIRTY_FILE)
    }
    /// The replay's state at the watermark, so the next sync needn't resolve the tree again.
    pub fn snapshot_path(&self) -> PathBuf {
        self.dir.join(SNAPSHOT_FILE)
    }
}

const SNAPSHOT_FILE: &str = "replay.snapshot";

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// `git fetch` each repository first. The only network action, and only on this flag.
    pub fetch: bool,
    /// Wait for a running sync instead of leaving the dirty flag.
    pub wait: bool,
    pub workers: usize,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            fetch: false,
            wait: false,
            workers: crate::default_workers(),
        }
    }
}

#[derive(Debug)]
// One value per sync: boxing the summary would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum SyncOutcome {
    Synced(SyncSummary),
    /// Another sync holds the lock; it will run one more round for this request (D2).
    AlreadyRunning,
}

/// What a sync did, summed over its rounds.
#[derive(Debug, Default, Clone)]
pub struct SyncSummary {
    pub rounds: usize,
    /// Steps written, including steps removed and walked again after a floor below the watermark.
    pub steps: usize,
    /// The last step kept when a round replayed from below the watermark.
    pub replayed_from: Option<i64>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub rewritten: Vec<String>,
    /// Package names declared in more than one repository (the first by name wins).
    pub duplicate_packages: Vec<String>,
    pub extract: ExtractStats,
    /// The last step after the sync, -1 for an empty index.
    pub watermark: i64,
    /// Repositories whose `git fetch` failed (`--fetch` only).
    pub fetch_failed: Vec<String>,
    /// The dirty flag was still set when the round cap stopped the sync.
    pub dirty_left: bool,
    /// The replay was restored from the last sync's snapshot instead of resolving the tree.
    pub restored: bool,
    /// Derived artefacts rebuilt (IDX-21): zero when nothing they read changed.
    pub derived: DerivedStats,
    /// Coverage at the last step (IDX-22): (files total, files parsed, imports total, imports
    /// resolved).
    pub coverage: (i64, i64, i64, i64),
    /// Why a subprocess adapter didn't run, or stopped (XS-24).
    pub adapter_notes: Vec<String>,
    pub rehydrate_ms: u128,
    pub walk_ms: u128,
    pub replay_ms: u128,
    pub total_ms: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("can't write the index")]
    Sqlite(#[from] rusqlite::Error),
    #[error("can't use {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0:?} can't be used as a repository name")]
    BadName(String),
}

impl Explain for SyncError {
    fn what(&self) -> String {
        match self {
            Self::Git(e) => e.what(),
            Self::Store(e) => e.what(),
            _ => self.to_string(),
        }
    }
    fn why(&self) -> String {
        match self {
            Self::Git(e) => e.why(),
            Self::Store(e) => e.why(),
            Self::Sqlite(e) => e.to_string(),
            Self::Io { source, .. } => source.to_string(),
            Self::BadName(_) => {
                "a repository name is a path prefix: letters, digits, '.', '_' and '-', unique"
                    .into()
            }
        }
    }
    fn action(&self) -> String {
        match self {
            Self::Git(e) => e.action(),
            Self::Store(e) => e.action(),
            Self::Sqlite(_) | Self::Io { .. } => {
                "check the free disk space and the data folder's permissions".into()
            }
            Self::BadName(_) => "rename the folder in the workspace file".into(),
        }
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> SyncError + '_ {
    move |source| SyncError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Creates the workspace's data folder, user-only.
pub fn create_dir(ws: &Workspace) -> Result<(), SyncError> {
    create_private_dir(&ws.dir).map_err(io(&ws.dir))
}

/// What [`bring_current`] found or did to an index file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upgrade {
    /// Already this binary's format (or not an index this binary can judge; opening reports it).
    Current,
    /// An older format, migrated by copy and rename (B3).
    Migrated,
    /// An older format with no migration path, or one that failed: build it again from git.
    Rebuild,
}

/// Brings an older index to this binary's format under the writer lock (IDX-16, B3): a migration
/// when one is registered, otherwise [`Upgrade::Rebuild`], since the index is a cache.
fn bring_current(lock: &WriterLock, path: &Path) -> Upgrade {
    // The probe skips quick_check, which a sync runs alongside its first reads; the rare file that
    // needs a migration is opened again with the check, which decides as it always has.
    let needs_migrate =
        |ix: Result<Index, _>| ix.is_ok_and(|ix| ix.state == cartograph_store::State::NeedsMigrate);
    if !needs_migrate(Index::probe(path)) || !needs_migrate(Index::open_read(path)) {
        return Upgrade::Current;
    }
    match cartograph_store::migrate::migrate(lock, path) {
        Ok(true) => Upgrade::Migrated,
        Ok(false) => Upgrade::Rebuild,
        Err(e) => {
            tracing::warn!(error = %e.what(), why = %e.why(), "the migration failed; rebuilding");
            Upgrade::Rebuild
        }
    }
}

/// Removes the index and everything derived from it, so the next round builds it from nothing.
fn discard(ws: &Workspace) -> Result<(), SyncError> {
    let path = ws.index_path();
    for p in [
        path.clone(),
        cartograph_store::sidecar(&path, "-wal"),
        cartograph_store::sidecar(&path, "-shm"),
        ws.snapshot_path(),
    ] {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(&p)(e)),
            _ => {}
        }
    }
    Ok(())
}

/// Migrates the workspace's index if it is in an older format, when no sync holds the lock (a
/// running sync migrates it itself). `serve` and `open` call it at start. On
/// [`Upgrade::Rebuild`] the index is left as it is: the caller starts a sync, which rebuilds it.
pub fn migrate_if_needed(ws: &Workspace) -> Result<Upgrade, SyncError> {
    // A read-only probe first: a current index never takes the writer lock, so a server start
    // doesn't make every workspace read as syncing for a moment.
    match Index::open_read(&ws.index_path()) {
        Ok(ix) if ix.state == cartograph_store::State::NeedsMigrate => {}
        _ => return Ok(Upgrade::Current),
    }
    let Some(lock) = WriterLock::try_acquire(&ws.dir).map_err(io(&ws.lock_path()))? else {
        return Ok(Upgrade::Current);
    };
    Ok(bring_current(&lock, &ws.index_path()))
}

/// Takes the writer lock (IDX-13), polling every 50 ms when `wait`.
fn acquire(ws: &Workspace, wait: bool) -> Result<Option<WriterLock>, SyncError> {
    loop {
        match WriterLock::try_acquire(&ws.dir).map_err(io(&ws.lock_path()))? {
            None if wait => std::thread::sleep(Duration::from_millis(50)),
            l => return Ok(l),
        }
    }
}

/// Brings the workspace's index up to each repository's HEAD (IDX-14), or, when another process
/// holds the writer lock, sets the dirty flag and returns at once (D2).
///
/// The flag is set before the lock is tried and checked again after the lock is released, so a
/// request that arrives while a sync is finishing is never lost: either the running sync sees the
/// flag, or the lock is free again when the requester tries it. At most [`MAX_EXTRA_ROUNDS`] extra
/// rounds run; a flag still set then is left for the next sync.
pub fn sync(
    ws: &Workspace,
    opts: &SyncOptions,
    cache: &mut FactCache,
) -> Result<SyncOutcome, SyncError> {
    sync_with_progress(ws, opts, cache, &mut |_, _| {})
}

/// [`sync`], calling `progress(done, total)` after every step of a round: the steps written so
/// far and the steps the round will write. The caller throttles; a round may call it thousands of
/// times.
pub fn sync_with_progress(
    ws: &Workspace,
    opts: &SyncOptions,
    cache: &mut FactCache,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<SyncOutcome, SyncError> {
    let t0 = Instant::now();
    let mut seen = HashSet::new();
    for r in &ws.repos {
        if !valid_repo_name(&r.name) || !seen.insert(r.name.as_str()) {
            return Err(SyncError::BadName(r.name.clone()));
        }
    }
    create_dir(ws)?;
    mark_dirty(&ws.dir).map_err(io(&ws.dirty_path()))?;
    let Some(mut lock) = acquire(ws, opts.wait)? else {
        return Ok(SyncOutcome::AlreadyRunning);
    };
    let mut s = SyncSummary::default();
    if opts.fetch {
        s.fetch_failed = fetch(ws);
    }
    let dirty = |l: &WriterLock| l.dir().join(DIRTY_FILE).exists();
    loop {
        // This round covers every request made before it started.
        lock.take_dirty().map_err(io(&ws.dirty_path()))?;
        round(&lock, ws, opts, cache, &mut s, progress)?;
        s.rounds += 1;
        let capped = s.rounds > MAX_EXTRA_ROUNDS;
        if lock.cancel_requested() || capped {
            break;
        }
        if dirty(&lock) {
            continue;
        }
        drop(lock);
        if !ws.dirty_path().exists() {
            break;
        }
        match acquire(ws, false)? {
            Some(l) => lock = l,
            // Whoever holds it now will see the flag.
            None => break,
        }
    }
    s.dirty_left = ws.dirty_path().exists();
    s.total_ms = t0.elapsed().as_millis();
    Ok(SyncOutcome::Synced(s))
}

/// C7, after "Split" added a `services:` entry for `repo` (IDX-20): rebuilds that repository's
/// structure and what depends on it (counts, search, layout) under the writer lock. Links and
/// lineages are untouched: this re-parents nodes, it doesn't replay.
pub fn restructure(ws: &Workspace, repo: &str) -> Result<DerivedStats, SyncError> {
    let Some(spec) = ws.repos.iter().find(|r| r.name == repo) else {
        return Err(SyncError::BadName(repo.to_owned()));
    };
    let _lock = acquire(ws, true)?.expect("acquire waits");
    let ix = Index::open_write(&ws.index_path())?;
    let conn = &ix.conn;
    let meta = |k: &str| -> rusqlite::Result<i64> {
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
            .optional()?
            .unwrap_or(0))
    };
    let (prefixed, watermark) = (meta("repo_prefix")? != 0, meta("watermark_seq")?);
    let row: Option<(i64, Option<Vec<u8>>)> = conn
        .query_row(
            "SELECT id, indexed_commit FROM repo WHERE name = ?1",
            [repo],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, Some(tip))) = row else {
        return Ok(DerivedStats::default());
    };
    let Some(tip) = oid(&tip) else {
        return Ok(DerivedStats::default());
    };
    let git = Repo::open(&spec.path)?;
    let batches = Batches::begin(conn)?;
    cartograph_store::derived::forget(conn, "detect", repo)?;
    let hash = cartograph_store::fact_hash(conn)?;
    let inputs = Inputs {
        repos: vec![RepoInput {
            id,
            name: &spec.name,
            prefixed,
            repo: &git,
            tip,
            scopes: &spec.scopes,
        }],
        services: &ws.services,
        contracts: &ws.contracts,
        environments: &ws.environments,
        default_environment: ws.default_environment.as_deref(),
        exclude: &ws.exclude,
        overrides: ws.overrides.as_deref(),
        config_hash: ws.config_hash.as_deref(),
        ..Default::default()
    };
    let d = refresh::<SyncError>(conn, &inputs, &hash, watermark)?;
    batches.finish(conn)?;
    Ok(d)
}

/// `git fetch` in each registry repository (IDX-14 security): no terminal prompt, `file://`
/// remotes only when the user's config allows them. Returns the repositories whose fetch failed;
/// the sync goes on with what is local.
///
/// Four repositories are fetched at a time (IDX-27, proposed); a failure is recorded and never
/// retried in a loop.
fn fetch(ws: &Workspace) -> Vec<String> {
    let mut failed = Vec::new();
    for chunk in ws.repos.chunks(4) {
        let ok: Vec<bool> = std::thread::scope(|s| {
            let jobs: Vec<_> = chunk
                .iter()
                .map(|r| {
                    s.spawn(move || {
                        Command::new("git")
                            .arg("-C")
                            .arg(&r.path)
                            .args(["-c", "protocol.file.allow=user", "fetch", "--quiet"])
                            .env("GIT_TERMINAL_PROMPT", "0")
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .status()
                            .is_ok_and(|s| s.success())
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap_or(false))
                .collect()
        });
        failed.extend(
            chunk
                .iter()
                .zip(ok)
                .filter(|(_, ok)| !ok)
                .map(|(r, _)| r.name.clone()),
        );
    }
    failed
}

struct DbCommit {
    seq: i64,
    repo: i64,
    sha: Vec<u8>,
    corrected: i64,
}

/// One repository's part of a round.
struct Part {
    spec: RepoSpec,
    repo: Repo,
    tip: ObjectId,
    id: Option<i64>,
    /// Its indexed steps, oldest first, as indexes into the round's commit list.
    indexed: Vec<usize>,
    /// How many of `indexed` are kept as the base of the walk.
    base: usize,
}

fn oid(bytes: &[u8]) -> Option<ObjectId> {
    ObjectId::try_from(bytes).ok()
}

/// The commits of `tip`'s first-parent chain.
fn first_parent_chain(repo: &Repo, tip: ObjectId) -> Result<HashSet<ObjectId>, GitError> {
    let obj = |e: Box<dyn std::error::Error + Send + Sync>| GitError::Object(e);
    let mut out = HashSet::new();
    for info in repo
        .local()
        .rev_walk([tip])
        .first_parent_only()
        .all()
        .map_err(|e| obj(Box::new(e)))?
    {
        out.insert(info.map_err(|e| obj(Box::new(e)))?.id);
    }
    Ok(out)
}

fn round(
    lock: &WriterLock,
    ws: &Workspace,
    opts: &SyncOptions,
    cache: &mut FactCache,
    s: &mut SyncSummary,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), SyncError> {
    let path = ws.index_path();
    let mut carried = None;
    if path.exists() && bring_current(lock, &path) == Upgrade::Rebuild {
        // Positions carry over by node key into the index built again (IDX-25).
        carried = Index::open_read(&path)
            .ok()
            .and_then(|ix| crate::layout::positions_by_key(&ix.conn).ok());
        discard(ws)?;
    }
    let (ix, mut check) = if path.exists() {
        Index::open_write_checking(&path)?
    } else {
        (Index::create(&path)?, None)
    };
    let r = round_on(
        lock, ws, opts, cache, s, progress, &ix.conn, carried, &mut check,
    );
    // A damaged index is reported as such, whatever else failed on it.
    if let Some(c) = check {
        c.wait()?;
    }
    r
}

/// Waits for the index's quick_check, if it is still running.
fn confirm(check: &mut Option<PendingCheck>) -> Result<(), SyncError> {
    match check.take() {
        Some(c) => Ok(c.wait()?),
        None => Ok(()),
    }
}

/// A round on the open index. `check` is its quick_check, still running: it is waited for before
/// anything is committed.
#[allow(clippy::too_many_arguments)]
fn round_on(
    lock: &WriterLock,
    ws: &Workspace,
    opts: &SyncOptions,
    cache: &mut FactCache,
    s: &mut SyncSummary,
    progress: &mut dyn FnMut(u64, u64),
    conn: &rusqlite::Connection,
    mut carried: Option<HashMap<String, crate::layout::Circle>>,
    check: &mut Option<PendingCheck>,
) -> Result<(), SyncError> {
    // `recover` commits at once, so the check comes first on an index that needs it.
    let state: Option<String> = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = 'state'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if matches!(state.as_deref(), Some("syncing" | "building")) {
        confirm(check)?;
    }
    recover(conn)?;
    let snapshot = take_snapshot(ws, conn)?;
    let mut batches = Batches::begin(conn)?;
    // A progress file left by the last run would read as this one's until its first step.
    let _ = lock.progress(
        &Progress {
            phase: "walk",
            done: 0,
            total: 0,
            repo: None,
        },
        true,
    );

    // 1. What is indexed.
    let db_repos: Vec<(i64, String)> = conn
        .prepare("SELECT id, name FROM repo ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let commits: Vec<DbCommit> = conn
        .prepare("SELECT seq, repo, sha, corrected_time FROM commits ORDER BY seq")?
        .query_map([], |r| {
            Ok(DbCommit {
                seq: r.get(0)?,
                repo: r.get(1)?,
                sha: r.get(2)?,
                corrected: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let name_of: HashMap<i64, &str> = db_repos.iter().map(|(i, n)| (*i, n.as_str())).collect();
    let watermark = commits.last().map_or(-1, |c| c.seq);
    let was_prefixed: Option<bool> = if commits.is_empty() {
        None
    } else {
        Some(
            conn.query_row(
                "SELECT value FROM meta WHERE key = 'repo_prefix'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
                != 0,
        )
    };
    let stored_config: Option<String> = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = 'walk_config'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let config = ws.walk_config();
    let mut specs = ws.repos.clone();
    specs.sort_by(|a, b| a.name.cmp(&b.name));
    let prefixed = was_prefixed.unwrap_or(false) || specs.len() > 1;
    // Every path changes when a one-repository index gains a second repository.
    // Every path changes when a one-repository index gains a second repository, and which paths
    // are walked changes with a branch, scope or exclude.
    let config_changed =
        !commits.is_empty() && walk_config_changed(stored_config.as_deref().unwrap_or(""), &config);
    // A one-repository index keeps bare paths. When that repository is replaced or renamed, the
    // new one's paths would land on the old one's rows (and the old one's removal step would close
    // them), so the history is walked again from nothing, as a full build of the new one is.
    let replaced = was_prefixed == Some(false)
        && !prefixed
        && specs.first().is_some_and(|s| {
            commits
                .iter()
                .any(|c| name_of.get(&c.repo).is_some_and(|n| *n != s.name))
        });
    let mut floor = if (was_prefixed == Some(false) && prefixed) || config_changed || replaced {
        -1
    } else {
        watermark
    };

    // 2. Each repository's base and tip.
    let mut parts = Vec::new();
    let mut rewrites: Vec<Rewrite> = Vec::new();
    for spec in specs {
        let repo = Repo::open(&spec.path)?;
        let tip = match &spec.branch {
            Some(b) => crate::freshness::branch_tip(&repo, b),
            None => repo.head(),
        };
        let Ok(tip) = tip else {
            // No commits yet: nothing to index, and not removed either.
            continue;
        };
        let id = db_repos.iter().find(|(_, n)| *n == spec.name).map(|r| r.0);
        let indexed: Vec<usize> = id.map_or_else(Vec::new, |id| {
            (0..commits.len())
                .filter(|i| commits[*i].repo == id)
                .collect()
        });
        let mut base = indexed.len();
        if let Some(&last) = indexed.last()
            && oid(&commits[last].sha) != Some(tip)
            && let Err(GitError::NotAncestor { .. }) =
                repo.walk_first_parent(oid(&commits[last].sha), tip, tracked)
        {
            // Rewritten (IDX-19): the merge base is the last indexed step still on the new
            // first-parent chain (none: the repo is walked again from its root). Steps after it
            // are replayed, for every repo, since cross-repo links depend on order.
            let chain = first_parent_chain(&repo, tip)?;
            base = indexed
                .iter()
                .take_while(|i| oid(&commits[**i].sha).is_some_and(|o| chain.contains(&o)))
                .count();
            floor = floor.min(commits[indexed[base]].seq - 1);
            s.rewritten.push(spec.name.clone());
            let kept = base.checked_sub(1).map(|i| &commits[indexed[i]]);
            rewrites.push(Rewrite {
                repo: id.expect("indexed"),
                old_head: commits[last].sha.clone(),
                new_head: tip,
                base: kept.map(|c| (c.sha.clone(), c.seq, c.corrected)),
            });
        }
        if id.is_none() {
            s.added.push(spec.name.clone());
        }
        parts.push(Part {
            spec,
            repo,
            tip,
            id,
            indexed,
            base,
        });
    }

    // 2b. An extractor whose version changed (IDX-21): its blobs are extracted again, and a blob
    // whose facts changed is replayed from the first step it was alive at.
    if let Some(changed_at) = reextract(conn, &parts, cache, opts.workers, &mut s.extract)? {
        floor = floor.min(changed_at - 1);
    }
    // The built-in adapters changed (XS-03): their facts are made again for every blob.
    let mut sub = ws.subs.session();
    if !ifaces::current(conn, &ws.subs)? {
        reextract_ifaces(conn, &parts, cache, &mut sub, &ws.subs)?;
    }

    // 3. The floor: the last indexed step before every new step. Walks start at the base; a part
    // whose indexed steps reach past the floor is walked again from the floor.
    let t = Instant::now();
    let walk = |p: &Part, base: usize| -> Result<Vec<CommitStep>, SyncError> {
        let from = base.checked_sub(1).map(|i| &commits[p.indexed[i]]);
        let globs: Vec<String> = ws.contracts.iter().map(|c| c.path.clone()).collect();
        let keep = |path: &str| p.spec.keeps_with(&ws.exclude, &globs, path);
        let mut w = p
            .repo
            .walk_first_parent(from.and_then(|c| oid(&c.sha)), p.tip, keep)?;
        if let Some(c) = from {
            w = w.with_prev_corrected(c.corrected);
        }
        Ok(w.collect::<Result<_, _>>()?)
    };
    let mut new: Vec<Vec<CommitStep>> = parts
        .iter()
        .map(|p| walk(p, p.base))
        .collect::<Result<_, _>>()?;
    let first_new = parts
        .iter()
        .zip(&new)
        .filter_map(|(p, n)| Some((n.first()?.corrected_time, p.spec.name.as_str())))
        .min();
    if let Some(key) = first_new {
        let before = commits.partition_point(|c| (c.corrected, name_of[&c.repo]) < key);
        floor = floor.min(before as i64 - 1);
    }
    for (p, n) in parts.iter_mut().zip(&mut new) {
        let keep = p
            .indexed
            .iter()
            .take(p.base)
            .take_while(|i| commits[**i].seq <= floor)
            .count();
        if keep < p.base {
            p.base = keep;
            *n = walk(p, keep)?;
        }
    }
    s.walk_ms += t.elapsed().as_millis();

    // 4. Truncate to the floor and rehydrate the replay there. A removed folder isn't walked
    // again, so its steps after the floor are read back from the index first and replayed with the
    // rest: its history is kept, and a new exclude applies to it as to every other repo.
    let t = Instant::now();
    let mut kept: Vec<(i64, Vec<CommitStep>)> = Vec::new();
    let mut kept_facts = Facts::new();
    if floor < watermark {
        if was_prefixed == Some(true) {
            for (id, name) in &db_repos {
                if !ws.repos.iter().any(|r| &r.name == name) {
                    let steps = removed_history(conn, *id, floor, &ws.exclude, &mut kept_facts)?;
                    if !steps.is_empty() {
                        kept.push((*id, steps));
                    }
                }
            }
        }
        for r in &rewrites {
            archive(conn, r)?;
        }
        truncate(conn, floor)?;
        s.replayed_from = Some(floor);
    }
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value)
         VALUES ('state', 'syncing'), ('repo_prefix', ?1), ('walk_config', ?2)",
        params![i64::from(prefixed), config],
    )?;
    if let Some(id) = ws.dir.file_name() {
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('workspace_id', ?1)",
            [id.to_string_lossy()],
        )?;
    }
    if let Some(h) = &ws.config_hash {
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('config_hash', ?1)",
            [h],
        )?;
    }
    let mut rows = Rows::load(conn, floor)?;
    let mut replay = Replay::default();
    let mut facts = Facts::new();
    if floor >= 0 {
        let tree = rehydrate(conn, &mut facts)?;
        match snapshot
            .filter(|(at, _)| *at == floor)
            .and_then(|(_, b)| Replay::restore(&tree, floor as u64, &b))
        {
            Some(r) => {
                replay = r;
                s.restored = true;
            }
            None => replay.apply(&tree, floor as u64),
        }
        replay.clear_events();
    }
    let indexed_facts: HashSet<(ObjectId, &str)> = facts.keys().copied().collect();
    for (k, f) in kept_facts {
        facts.entry(k).or_insert(f);
    }
    s.rehydrate_ms += t.elapsed().as_millis();

    // 5. Extract, then write repos and facts first: a batch's file versions never point at a blob
    // without facts.
    let t = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.workers.max(1))
        .build()
        .expect("a thread pool");
    let mut ids = Vec::new();
    let mut ifs = ifaces::Ifaces::new();
    let mut ifs_seen = HashSet::new();
    for (p, n) in parts.iter().zip(&new) {
        let items = n
            .iter()
            .flat_map(|s| &s.changes)
            .filter_map(|c| Some((c.new?, extractor_for(&c.path)?.1)));
        let st = extract(&p.repo, items, cache, &pool, &mut facts)?;
        let paths = n
            .iter()
            .flat_map(|s| &s.changes)
            .filter_map(|c| Some((c.new?, c.path.clone())));
        ifaces::extract(&p.repo, paths, cache, &mut ifs, &mut ifs_seen, &mut sub)?;
        s.extract.blobs += st.blobs;
        s.extract.parsed += st.parsed;
        s.extract.from_cache += st.from_cache;
        s.extract.skipped += st.skipped;
        let path = p.spec.path.display().to_string();
        let shallow = p.repo.is_shallow();
        ids.push(match p.id {
            Some(id) => {
                conn.execute(
                    "UPDATE repo SET path = ?2, shallow = ?3, removed = 0 WHERE id = ?1",
                    params![id, path, shallow],
                )?;
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO repo (name, path, shallow) VALUES (?1, ?2, ?3)",
                    params![p.spec.name, path, shallow],
                )?;
                conn.last_insert_rowid()
            }
        });
    }
    rows.facts(facts.iter().filter(|(k, _)| !indexed_facts.contains(*k)))?;
    ifaces::write(conn, &ifs, &ws.subs)?;
    ifaces::mark_current(conn, &ws.subs)?;
    s.adapter_notes.append(&mut sub.notes);
    drop(sub);

    // 6. Replay the merged new steps, in batches. The first batch may commit: the index must have
    // passed its check by then.
    confirm(check)?;
    let mut prefixes: Vec<Option<&str>> = parts
        .iter()
        .map(|p| prefixed.then_some(p.spec.name.as_str()))
        .collect();
    // A removed folder's steps carry their namespaced paths already.
    for (id, steps) in kept {
        new.push(steps);
        ids.push(id);
        prefixes.push(None);
    }
    let mut seq = floor;
    let total = new.iter().map(Vec::len).sum::<usize>() as u64;
    let mut done = 0;
    let mut cancelled = false;
    for (r, step) in merge(new) {
        seq += 1;
        let prefix = prefixes[r];
        let path = |p: &str| namespaced(prefix, p);
        let changes: Vec<Change> = step
            .changes
            .iter()
            .map(|c| {
                let f = c.new.and_then(|o| {
                    let kind = extractor_for(&c.path)?.1;
                    facts.get(&(o, kind)).map(Arc::clone)
                });
                (path(&c.path), f)
            })
            .collect();
        if is_removal(&step) {
            rows.commit_only(seq, ids[r], &step)?;
        } else {
            rows.commit(seq, ids[r], &step)?;
        }
        rows.step(seq, ids[r], &step, &path, &changes, &mut replay)?;
        s.steps += 1;
        done += 1;
        progress(done, total);
        // Readers in other processes (the app's freshness) see how far this run is.
        let _ = lock.progress(
            &Progress {
                phase: "replay",
                done,
                total,
                repo: None,
            },
            false,
        );
        if batches.step() {
            rows.flush(seq)?;
            batches.commit(conn)?;
            // E5: a cancel stops the round after the batch it lands in, not at its end. The steps
            // so far are a whole prefix of the merged timeline, so the round ends below as an
            // earlier, complete index (behind) and the next sync carries on from it.
            if lock.cancel_requested() {
                cancelled = true;
                break;
            }
        }
    }
    // A removed folder: one synthetic step deletes its live files, so its intervals close. After a
    // cancel that waits for the next round.
    let removed: Vec<(i64, String)> = db_repos
        .iter()
        .filter(|(_, n)| !cancelled && !ws.repos.iter().any(|r| &r.name == n))
        .cloned()
        .collect();
    for (id, name) in &removed {
        let live: Vec<String> = conn
            .prepare(
                "SELECT fp.path FROM file_path fp JOIN file f ON f.id = fp.file
                 WHERE f.repo = ?1 AND fp.to_seq IS NULL ORDER BY fp.path",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        conn.execute("UPDATE repo SET removed = 1 WHERE id = ?1", [id])?;
        if live.is_empty() {
            continue;
        }
        seq += 1;
        let corrected = conn.query_row(
            "SELECT coalesce(max(corrected_time), 0) + 1 FROM commits",
            [],
            |r| r.get::<_, i64>(0),
        )?;
        let step = removal_step(seq, *id, name, corrected, &live);
        rows.commit_only(seq, *id, &step)?;
        let changes: Vec<Change> = live.iter().map(|p| (p.clone(), None)).collect();
        rows.step(
            seq,
            *id,
            &step,
            &|p: &str| p.to_owned(),
            &changes,
            &mut replay,
        )?;
        s.removed.push(name.clone());
        s.steps += 1;
    }
    rows.flush(seq)?;
    // The check reads each path's first folder as its repo; a one-repository tree has no repo
    // prefix, so its top folders would pass for repos and two packages in one repo would warn.
    s.duplicate_packages = if prefixed {
        duplicate_packages(replay.tree())
    } else {
        Vec::new()
    };
    s.watermark = seq;
    s.replay_ms += t.elapsed().as_millis();
    s.coverage = coverage(&replay);
    // The replay is done with: its snapshot is encoded, and it and the facts are freed, on another
    // thread while this one writes the derived rows. The file is written after the commit below.
    let snapshot_body = std::thread::spawn(move || {
        let body = (seq >= 0).then(|| replay.snapshot());
        drop((replay, facts));
        body
    });
    let (files_total, files_parsed, imports_total, imports_resolved) = s.coverage;
    conn.execute(
        "INSERT INTO run (started_at, finished_at, writer_version, stats, files_total,
           files_parsed, imports_total, imports_resolved)
         VALUES (unixepoch(), unixepoch(), ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            env!("CARGO_PKG_VERSION"),
            format!("sync: {} steps after {floor}", s.steps),
            files_total,
            files_parsed,
            imports_total,
            imports_resolved
        ],
    )?;
    conn.execute("UPDATE meta SET value = 'complete' WHERE key = 'state'", [])?;
    // CARTOGRAPH.md (SRV-23) is written by the caller from `freshness`.
    let hash = store_fact_hash(conn)?;
    let moved = std::mem::take(&mut rows.moved);
    drop(rows);
    let tips: Vec<Option<ObjectId>> = parts
        .iter()
        .map(|p| {
            conn.query_row(
                "SELECT indexed_commit FROM repo WHERE name = ?1",
                [&p.spec.name],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .map(|b| b.as_deref().and_then(oid))
        })
        .collect::<Result<_, _>>()?;
    let inputs = Inputs {
        repos: parts
            .iter()
            .zip(&ids)
            .zip(&tips)
            .filter_map(|((p, id), tip)| {
                Some(RepoInput {
                    id: *id,
                    name: &p.spec.name,
                    prefixed,
                    repo: &p.repo,
                    tip: (*tip)?,
                    scopes: &p.spec.scopes,
                })
            })
            .collect(),
        services: &ws.services,
        contracts: &ws.contracts,
        environments: &ws.environments,
        default_environment: ws.default_environment.as_deref(),
        exclude: &ws.exclude,
        overrides: ws.overrides.as_deref(),
        config_hash: ws.config_hash.as_deref(),
        moved,
        carried: carried.take(),
        removed: db_repos
            .iter()
            .filter(|(_, n)| !ws.repos.iter().any(|r| &r.name == n))
            .map(|(id, n)| (*id, n.clone(), prefixed))
            .collect(),
    };
    let d = refresh::<SyncError>(conn, &inputs, &hash, seq)?;
    s.derived.structures += d.structures;
    s.derived.counts += d.counts;
    s.derived.search_rows += d.search_rows;
    s.derived.layouts += d.layouts;
    batches.finish(conn)?;
    if let Ok(Some(body)) = snapshot_body.join() {
        put_snapshot(ws, &hash.to_string(), seq, &body);
    }
    Ok(())
}

/// Extracts again the blobs whose facts came from an older version of their extractor (IDX-21,
/// B3), reading each from a repository that has it. Only those extractors' blobs are parsed; facts
/// and cache rows of every other extractor are untouched. The fact rows move to the new version.
/// Returns the first step at which a blob whose facts changed was alive, if any.
fn reextract(
    conn: &rusqlite::Connection,
    parts: &[Part],
    cache: &mut FactCache,
    workers: usize,
    stats: &mut ExtractStats,
) -> Result<Option<i64>, SyncError> {
    let stale: Vec<(i64, String)> = conn
        .prepare("SELECT id, name, version FROM extractor")?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .filter_map(|r| match r {
            Ok((id, name, version)) => {
                let current = extractor_for_kind(&name)?;
                (current != version).then_some(Ok((id, name)))
            }
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<_, _>>()?;
    if stale.is_empty() {
        return Ok(None);
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers.max(1))
        .build()
        .expect("a thread pool");
    let mut first: Option<i64> = None;
    for (old_id, kind) in stale {
        let Some(kind) = cartograph_parse::EXTRACTORS
            .iter()
            .find_map(|e| e.kinds().iter().find(|k| **k == kind).copied())
        else {
            continue;
        };
        // Each blob with one repository it was seen in.
        let blobs: Vec<(Vec<u8>, String, Vec<u8>)> = conn
            .prepare(
                "SELECT f.blob, (SELECT r.name FROM file_version fv JOIN file fi ON fi.id = fv.file
                                 JOIN repo r ON r.id = fi.repo WHERE fv.blob = f.blob LIMIT 1),
                        f.payload
                 FROM fact f WHERE f.extractor = ?1",
            )?
            .query_map([old_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get(2)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        let new_id = crate::rows::extractor_id(conn, kind)?;
        for p in parts {
            let mine: Vec<&(Vec<u8>, String, Vec<u8>)> =
                blobs.iter().filter(|b| b.1 == p.spec.name).collect();
            let items: Vec<(ObjectId, &'static str)> = mine
                .iter()
                .filter_map(|b| Some((oid(&b.0)?, kind)))
                .collect();
            let mut fresh = Facts::new();
            let st = extract(&p.repo, items, cache, &pool, &mut fresh)?;
            stats.blobs += st.blobs;
            stats.parsed += st.parsed;
            stats.from_cache += st.from_cache;
            stats.skipped += st.skipped;
            for (blob, _, old) in mine {
                let Some(f) = oid(blob).and_then(|o| fresh.get(&(o, kind))) else {
                    continue;
                };
                let payload = f.encode();
                conn.prepare_cached(
                    "UPDATE fact SET extractor = ?3, payload = ?4 WHERE blob = ?1 AND extractor = ?2",
                )?
                .execute(params![blob, old_id, new_id, payload])?;
                if &payload != old {
                    let at: Option<i64> = conn
                        .prepare_cached("SELECT min(from_seq) FROM file_version WHERE blob = ?1")?
                        .query_row([blob], |r| r.get(0))?;
                    first = match (first, at) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
            }
        }
        conn.execute(
            "DELETE FROM extractor WHERE id = ?1 AND id NOT IN (SELECT extractor FROM fact)",
            [old_id],
        )?;
    }
    Ok(first)
}

/// Interface facts for every blob a repository has, after the adapters changed or in an index that
/// never had them. A repository dropped from the workspace has no folder to read, so its blobs
/// have none until it is back.
fn reextract_ifaces(
    conn: &rusqlite::Connection,
    parts: &[Part],
    cache: &mut FactCache,
    sub: &mut cartograph_facts::adapters::subprocess::Session,
    subs: &cartograph_facts::adapters::subprocess::SubSet,
) -> Result<(), SyncError> {
    conn.execute(
        "DELETE FROM fact WHERE extractor IN (SELECT id FROM extractor WHERE name = ?1)",
        [ifaces::EXTRACTOR],
    )?;
    conn.execute("DELETE FROM extractor WHERE name = ?1", [ifaces::EXTRACTOR])?;
    let mut found = ifaces::Ifaces::new();
    let mut seen = HashSet::new();
    for p in parts {
        let items: Vec<(ObjectId, String)> = conn
            .prepare(
                "SELECT fv.blob, (SELECT path FROM file_path fp WHERE fp.file = fv.file
                                  ORDER BY fp.from_seq DESC LIMIT 1)
                 FROM file_version fv JOIN file f ON f.id = fv.file JOIN repo r ON r.id = f.repo
                 WHERE r.name = ?1",
            )?
            .query_map([&p.spec.name], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .filter_map(|r| match r {
                Ok((b, Some(path))) => oid(&b).map(|o| Ok((o, path))),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<_, _>>()?;
        ifaces::extract(&p.repo, items, cache, &mut found, &mut seen, sub)?;
    }
    // Every adapter fact is made again: the marker count starts over.
    conn.execute(
        "UPDATE meta SET value = 0 WHERE key IN (?1, ?2)",
        [ifaces::CODEGEN_BLOBS, ifaces::EVENT_BLOBS],
    )?;
    ifaces::write(conn, &found, subs)?;
    Ok(())
}

/// The version of the extractor that makes `kind`, if this binary has one.
fn extractor_for_kind(kind: &str) -> Option<&'static str> {
    cartograph_parse::EXTRACTORS
        .iter()
        .find(|e| kind.split('/').next() == Some(e.name()))
        .map(|e| e.version())
}

/// The first line of a snapshot file: what it was taken from. A snapshot is used only by the next
/// round, at the same step, of an index with the same format and fact hash (which covers every
/// fact, lineage and interval), written by a replay of the same snapshot version.
fn snapshot_header(fact_hash: &str, seq: i64) -> String {
    format!(
        "cartograph-replay {SNAPSHOT_VERSION} {FORMAT_VERSION} {fact_hash} {seq}\n",
        FORMAT_VERSION = cartograph_store::FORMAT_VERSION
    )
}

/// Reads and removes the last sync's snapshot: the step it was taken at and its body, if it still
/// matches the index. Removed before anything is written, so a round that dies part way never
/// leaves a snapshot of a state the index no longer has.
fn take_snapshot(
    ws: &Workspace,
    conn: &rusqlite::Connection,
) -> Result<Option<(i64, Vec<u8>)>, SyncError> {
    let path = ws.snapshot_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(None);
    };
    if std::fs::remove_file(&path).is_err() {
        return Ok(None);
    }
    let meta = |k: &str| -> rusqlite::Result<Option<String>> {
        conn.query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1",
            [k],
            |r| r.get(0),
        )
        .optional()
    };
    let (Some(hash), Some(seq), Some("complete")) = (
        meta("fact_hash")?,
        meta("watermark_seq")?,
        meta("state")?.as_deref(),
    ) else {
        return Ok(None);
    };
    let Ok(seq) = seq.parse::<i64>() else {
        return Ok(None);
    };
    let header = snapshot_header(&hash, seq);
    Ok(bytes
        .strip_prefix(header.as_bytes())
        .map(|body| (seq, body.to_vec())))
}

/// Best effort: without a snapshot the next sync resolves the tree again, which is slower, not
/// wrong.
fn put_snapshot(ws: &Workspace, fact_hash: &str, seq: i64, body: &[u8]) {
    let mut bytes = snapshot_header(fact_hash, seq).into_bytes();
    bytes.extend_from_slice(body);
    // No fsync: the snapshot is a cache. A torn file fails its decode or the tree check in
    // `Replay::restore`, and the sync then resolves the tree again.
    if let Err(e) = write_cache(&ws.snapshot_path(), &bytes) {
        tracing::warn!(error = %e, "can't write the replay snapshot; the next sync resolves the tree again");
    }
}

/// Writes `bytes` user-only through a temp file and a rename, without fsync.
fn write_cache(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(&tmp)?.write_all(bytes)?;
    std::fs::rename(&tmp, path)
}

/// The synthetic step of a folder removed from the workspace: it deletes the folder's live files.
/// Its sha is 20 bytes no git object can have by accident: a marker and the step.
fn removal_step(seq: i64, repo: i64, name: &str, corrected: i64, live: &[String]) -> CommitStep {
    let mut sha = [0xffu8; 20];
    sha[..8].copy_from_slice(&seq.to_be_bytes());
    sha[8..16].copy_from_slice(&repo.to_be_bytes());
    CommitStep {
        sha: ObjectId::from_bytes_or_panic(&sha),
        time: corrected,
        corrected_time: corrected,
        author: "cartograph".into(),
        summary: format!("{name}: removed from workspace"),
        changes: live
            .iter()
            .map(|p| cartograph_git::Change {
                path: p.clone(),
                old: None,
                new: None,
            })
            .collect(),
        renames: Vec::new(),
        rename_limit: false,
    }
}

/// The synthetic step that closed a removed folder's files ([`removal_step`]): it is not a
/// commit of the repository, so it never becomes the repository's indexed commit.
fn is_removal(step: &CommitStep) -> bool {
    step.author == "cartograph" && step.sha.as_bytes()[16..] == [0xff; 4]
}

/// A removed repository's steps after `floor`, oldest first, rebuilt from its rows: its commits,
/// and at each one the paths and blobs of its lineages before and after, so additions,
/// modifications, deletions and renames come back as they were walked. Paths are namespaced as
/// stored; those the workspace now excludes are left out. The facts of every blob they add go into
/// `facts`.
fn removed_history(
    conn: &rusqlite::Connection,
    repo: i64,
    floor: i64,
    exclude: &[String],
    facts: &mut Facts,
) -> rusqlite::Result<Vec<CommitStep>> {
    use cartograph_git::{Change as GitChange, Rename};
    use std::collections::BTreeMap;
    type Span<T> = (T, i64, Option<i64>);
    // Per lineage: its paths (path, via, similarity) and its versions, each with its interval.
    type PathRow = (String, String, Option<i64>);
    let mut paths: BTreeMap<i64, Vec<Span<PathRow>>> = BTreeMap::new();
    let mut versions: BTreeMap<i64, Vec<Span<ObjectId>>> = BTreeMap::new();
    let mut st = conn.prepare(
        "SELECT fp.file, fp.path, fp.via, fp.similarity, fp.from_seq, fp.to_seq FROM file_path fp
         JOIN file f ON f.id = fp.file WHERE f.repo = ?1 AND (fp.from_seq > ?2 OR fp.to_seq > ?2
           OR fp.to_seq IS NULL)",
    )?;
    let mut rows = st.query(params![repo, floor])?;
    while let Some(r) = rows.next()? {
        paths.entry(r.get(0)?).or_default().push((
            (r.get(1)?, r.get(2)?, r.get(3)?),
            r.get(4)?,
            r.get(5)?,
        ));
    }
    let mut st = conn.prepare(
        "SELECT fv.file, fv.blob, fv.from_seq, fv.to_seq FROM file_version fv
         JOIN file f ON f.id = fv.file WHERE f.repo = ?1 AND (fv.from_seq > ?2 OR fv.to_seq > ?2
           OR fv.to_seq IS NULL)",
    )?;
    let mut rows = st.query(params![repo, floor])?;
    while let Some(r) = rows.next()? {
        if let Some(b) = oid(r.get_ref(1)?.as_blob()?) {
            versions
                .entry(r.get(0)?)
                .or_default()
                .push((b, r.get(2)?, r.get(3)?));
        }
    }
    let at = |spans: &[(i64, Option<i64>)], t: i64| {
        spans
            .iter()
            .position(|(f, to)| *f <= t && to.is_none_or(|to| t < to))
    };
    let kept = |p: &str| {
        let inner = p.split_once('/').map_or(p, |(_, p)| p);
        !crate::timeline::excluded(exclude, inner)
    };
    let mut steps_at: BTreeMap<i64, (Vec<GitChange>, Vec<Rename>)> = BTreeMap::new();
    for (file, ps) in &paths {
        let vs = versions.get(file).map_or(&[][..], Vec::as_slice);
        let mut events: Vec<i64> = ps
            .iter()
            .flat_map(|(_, f, t)| [Some(*f), *t])
            .chain(vs.iter().flat_map(|(_, f, t)| [Some(*f), *t]))
            .flatten()
            .filter(|e| *e > floor)
            .collect();
        events.sort_unstable();
        events.dedup();
        let pspans: Vec<_> = ps.iter().map(|(_, f, t)| (*f, *t)).collect();
        let vspans: Vec<_> = vs.iter().map(|(_, f, t)| (*f, *t)).collect();
        let state = |t: i64| {
            let p = at(&pspans, t).map(|i| &ps[i].0);
            let v = at(&vspans, t).map(|i| vs[i].0);
            p.zip(v).filter(|((path, ..), _)| kept(path))
        };
        for e in events {
            let (before, after) = (state(e - 1), state(e));
            let (changes, renames) = steps_at.entry(e).or_default();
            let mut push = |path: &str, old, new| {
                changes.push(GitChange {
                    path: path.to_owned(),
                    old,
                    new,
                })
            };
            match (before, after) {
                (Some(((a, ..), x)), Some(((b, via, sim), y))) if a != b => {
                    push(a, Some(x), None);
                    push(b, None, Some(y));
                    renames.push(Rename {
                        from: a.clone(),
                        to: b.clone(),
                        similarity: sim.map_or(100, |s| s.clamp(0, 100) as u8),
                        exact: via == "rename-exact",
                    });
                }
                (Some(((a, ..), x)), Some((_, y))) if x != y => push(a, Some(x), Some(y)),
                (Some(((a, ..), x)), None) => push(a, Some(x), None),
                (None, Some(((b, ..), y))) => push(b, None, Some(y)),
                _ => {}
            }
        }
    }
    for (changes, _) in steps_at.values() {
        for c in changes {
            let Some(blob) = c.new else { continue };
            if let Some((_, kind)) = extractor_for(&c.path)
                && !facts.contains_key(&(blob, kind))
                && let Some(f) = conn
                    .prepare_cached(
                        "SELECT f.payload FROM fact f JOIN extractor e ON e.id = f.extractor
                         WHERE f.blob = ?1 AND e.name = ?2",
                    )?
                    .query_row(params![blob.as_bytes(), kind], |r| r.get::<_, Vec<u8>>(0))
                    .optional()?
                    .and_then(|b| BlobFacts::decode(&b))
            {
                facts.insert((blob, kind), Arc::new(f));
            }
        }
    }
    let mut st = conn.prepare(
        "SELECT seq, sha, time, corrected_time, author, summary FROM commits
         WHERE repo = ?1 AND seq > ?2 ORDER BY seq",
    )?;
    let mut rows = st.query(params![repo, floor])?;
    let mut steps = Vec::new();
    while let Some(r) = rows.next()? {
        let seq: i64 = r.get(0)?;
        let Some(sha) = oid(r.get_ref(1)?.as_blob()?) else {
            continue;
        };
        let (mut changes, mut renames) = steps_at.remove(&seq).unwrap_or_default();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        renames.sort_by(|a, b| a.to.cmp(&b.to));
        steps.push(CommitStep {
            sha,
            time: r.get(2)?,
            corrected_time: r.get(3)?,
            author: r.get(4)?,
            summary: r.get(5)?,
            changes,
            renames,
            rename_limit: false,
        });
    }
    Ok(steps)
}

/// A history rewrite found by a round (IDX-19).
struct Rewrite {
    repo: i64,
    old_head: Vec<u8>,
    new_head: ObjectId,
    /// The merge base: its sha, step and corrected time; `None` for unrelated history.
    base: Option<(Vec<u8>, i64, i64)>,
}

/// Archived intervals kept per rewrite (IDX-19 security, proposed); the rest are counted.
pub const ARCHIVE_CAP: i64 = 10_000;

/// Records a rewrite and keeps the links of the old history that opened after the merge base,
/// keyed by the old commits' shas (board G10: "links that existed only in the old history are kept
/// separately"). Runs before the truncation removes them.
fn archive(conn: &rusqlite::Connection, r: &Rewrite) -> rusqlite::Result<()> {
    let after = r.base.as_ref().map_or(-1, |b| b.1);
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM interval i JOIN link l ON l.id = i.link JOIN node n ON n.id = l.src
         JOIN file f ON f.id = n.file WHERE f.repo = ?1 AND i.from_seq > ?2",
        params![r.repo, after],
        |x| x.get(0),
    )?;
    let archived = total.min(ARCHIVE_CAP);
    conn.execute(
        "INSERT INTO rewrite (repo, old_head, new_head, base, at, recorded_at, archived, dropped)
         VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), ?6, ?7)",
        params![
            r.repo,
            r.old_head,
            r.new_head.as_bytes(),
            r.base.as_ref().map(|b| b.0.clone()),
            r.base.as_ref().map(|b| b.2),
            archived,
            total - archived
        ],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO archived_interval (rewrite, link, from_sha, to_sha)
         SELECT ?1, i.link, cf.sha, ct.sha FROM interval i JOIN link l ON l.id = i.link
         JOIN node n ON n.id = l.src JOIN file f ON f.id = n.file
         JOIN commits cf ON cf.seq = i.from_seq LEFT JOIN commits ct ON ct.seq = i.to_seq
         WHERE f.repo = ?2 AND i.from_seq > ?3 ORDER BY i.from_seq, i.link LIMIT ?4",
        params![id, r.repo, after, ARCHIVE_CAP],
    )?;
    Ok(())
}

/// Removes every step after `floor`: rows opened after it go, rows closed after it reopen.
fn truncate(conn: &rusqlite::Connection, floor: i64) -> rusqlite::Result<()> {
    for t in ["interval", "file_version", "file_path", "unresolved"] {
        conn.execute(&format!("DELETE FROM {t} WHERE from_seq > ?1"), [floor])?;
        conn.execute(
            &format!("UPDATE {t} SET to_seq = NULL WHERE to_seq > ?1"),
            [floor],
        )?;
    }
    conn.execute("DELETE FROM commits WHERE seq > ?1", [floor])?;
    conn.execute("DELETE FROM note WHERE seq > ?1", [floor])?;
    // Facts of blobs only the removed steps had, as a full build would never have seen them.
    conn.execute_batch(
        "DELETE FROM fact WHERE blob NOT IN (SELECT blob FROM file_version);
         DELETE FROM extractor WHERE id NOT IN (SELECT extractor FROM fact);",
    )?;
    // head moves with indexed_commit, as Rows::flush writes them: freshness and the query layer
    // read head as the indexed commit, and a repo moved back gets no step to rewrite it.
    conn.execute(
        "UPDATE repo SET
           indexed_commit = (SELECT sha FROM commits c WHERE c.repo = repo.id ORDER BY seq DESC LIMIT 1),
           head = (SELECT sha FROM commits c WHERE c.repo = repo.id ORDER BY seq DESC LIMIT 1),
           indexed_seq = (SELECT max(seq) FROM commits c WHERE c.repo = repo.id)",
        [],
    )?;
    Ok(())
}

/// The tree live at the watermark with each file's facts, read from the index: open
/// `file_version` rows and their `fact` rows, in one ordered pass.
fn rehydrate(conn: &rusqlite::Connection, facts: &mut Facts) -> rusqlite::Result<Vec<Change>> {
    let mut st = conn.prepare(
        "SELECT fp.path, fv.blob, e.name, f.payload FROM file_version fv
         JOIN file_path fp ON fp.file = fv.file AND fp.to_seq IS NULL
         JOIN fact f ON f.blob = fv.blob JOIN extractor e ON e.id = f.extractor
         WHERE fv.to_seq IS NULL ORDER BY fp.path",
    )?;
    let mut rows = st.query([])?;
    let mut tree = Vec::new();
    while let Some(r) = rows.next()? {
        let path: String = r.get(0)?;
        let Some((_, kind)) = extractor_for(&path) else {
            continue;
        };
        if r.get_ref(2)?.as_str()? != kind {
            continue;
        }
        let Some(o) = oid(r.get_ref(1)?.as_blob()?) else {
            continue;
        };
        let f = match facts.get(&(o, kind)) {
            Some(f) => Arc::clone(f),
            None => {
                let Some(decoded) = BlobFacts::decode(r.get_ref(3)?.as_blob()?) else {
                    continue;
                };
                let f = Arc::new(decoded);
                facts.insert((o, kind), Arc::clone(&f));
                f
            }
        };
        tree.push((path, Some(f)));
    }
    Ok(tree)
}
