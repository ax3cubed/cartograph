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
use cartograph_facts::{FactCache, Interval, Link, Replay, SNAPSHOT_VERSION};
use cartograph_git::{CommitStep, GitError, ObjectId, Repo};
use cartograph_parse::{BlobFacts, extractor_for, tracked};
use cartograph_store::hash::store_fact_hash;
use cartograph_store::rusqlite::{self, OptionalExtension, params};
use cartograph_store::writer::{Batches, DIRTY_FILE, Progress, WriterLock, mark_dirty, recover};
use cartograph_store::{Index, StoreError};

use crate::extract::{ExtractStats, Facts, extract, extractor_of};
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
}

impl Workspace {
    /// What decides which paths a walk keeps: the excludes on the first line, then one line per
    /// repository (name, branch, scopes).
    fn walk_config(&self) -> String {
        let mut ex = self.exclude.clone();
        ex.sort();
        let mut lines = vec![ex.join("\0")];
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
    match Index::open_read(path) {
        Ok(ix) if ix.state == cartograph_store::State::NeedsMigrate => {}
        _ => return Upgrade::Current,
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

/// `git fetch` in each registry repository (IDX-14 security): no terminal prompt, `file://`
/// remotes only when the user's config allows them. Returns the repositories whose fetch failed;
/// the sync goes on with what is local.
fn fetch(ws: &Workspace) -> Vec<String> {
    let mut failed = Vec::new();
    for r in &ws.repos {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&r.path)
            .args(["-c", "protocol.file.allow=user", "fetch", "--quiet"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            failed.push(r.name.clone());
        }
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
    if path.exists() && bring_current(lock, &path) == Upgrade::Rebuild {
        discard(ws)?;
    }
    let ix = if path.exists() {
        Index::open_write(&path)?
    } else {
        Index::create(&path)?
    };
    let conn = &ix.conn;
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
            // Rewritten: keep the indexed steps still on the chain, replay from there.
            let chain = first_parent_chain(&repo, tip)?;
            base = indexed
                .iter()
                .take_while(|i| oid(&commits[**i].sha).is_some_and(|o| chain.contains(&o)))
                .count();
            floor = floor.min(commits[indexed[base]].seq - 1);
            s.rewritten.push(spec.name.clone());
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

    // 3. The floor: the last indexed step before every new step. Walks start at the base; a part
    // whose indexed steps reach past the floor is walked again from the floor.
    let t = Instant::now();
    let walk = |p: &Part, base: usize| -> Result<Vec<CommitStep>, SyncError> {
        let from = base.checked_sub(1).map(|i| &commits[p.indexed[i]]);
        let keep = |path: &str| p.spec.keeps(&ws.exclude, path);
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
            None => {
                replay.apply(&tree, floor as u64);
                replay.closed.clear();
            }
        }
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
    for (p, n) in parts.iter().zip(&new) {
        let items = n
            .iter()
            .flat_map(|s| &s.changes)
            .filter_map(|c| Some((c.new?, extractor_for(&c.path)?.1)));
        let st = extract(&p.repo, items, cache, &pool, &mut facts)?;
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

    // 6. Replay the merged new steps, in batches.
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
        let changes: Vec<Change> = step
            .changes
            .iter()
            .map(|c| {
                let f = c.new.and_then(|o| {
                    let kind = extractor_for(&c.path)?.1;
                    facts.get(&(o, kind)).map(Arc::clone)
                });
                (namespaced(prefixes[r], &c.path), f)
            })
            .collect();
        if is_removal(&step) {
            rows.commit_only(seq, ids[r], &step)?;
        } else {
            rows.commit(seq, ids[r], &step)?;
        }
        replay.apply(&changes, seq as u64);
        for ((path, _), c) in changes.iter().zip(&step.changes) {
            rows.file_change(ids[r], seq, path, c.new)?;
        }
        for i in replay.closed.drain(..) {
            rows.close(&i)?;
        }
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
            rows.flush(&replay, seq)?;
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
        rows.removal(seq, *id, name, corrected)?;
        let changes: Vec<Change> = live.iter().map(|p| (p.clone(), None)).collect();
        replay.apply(&changes, seq as u64);
        for p in &live {
            rows.file_change(*id, seq, p, None)?;
        }
        for i in replay.closed.drain(..) {
            rows.close(&i)?;
        }
        s.removed.push(name.clone());
        s.steps += 1;
    }
    rows.flush(&replay, seq)?;
    // The check reads each path's first folder as its repo; a one-repository tree has no repo
    // prefix, so its top folders would pass for repos and two packages in one repo would warn.
    s.duplicate_packages = if prefixed {
        duplicate_packages(replay.tree())
    } else {
        Vec::new()
    };
    s.watermark = seq;
    s.replay_ms += t.elapsed().as_millis();
    conn.execute(
        "INSERT INTO run (started_at, finished_at, writer_version, stats)
         VALUES (unixepoch(), unixepoch(), ?1, ?2)",
        params![
            env!("CARGO_PKG_VERSION"),
            format!("sync: {} steps after {floor}", s.steps)
        ],
    )?;
    conn.execute("UPDATE meta SET value = 'complete' WHERE key = 'state'", [])?;
    // ponytail: IDX-21 (derived rows) and IDX-25 (layout) join here; CARTOGRAPH.md (SRV-23) is
    // written by the caller from `freshness`.
    let hash = store_fact_hash(conn)?;
    batches.finish(conn)?;
    put_snapshot(ws, &hash.to_string(), seq, &replay);
    Ok(())
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
fn put_snapshot(ws: &Workspace, fact_hash: &str, seq: i64, replay: &Replay) {
    if seq < 0 {
        return;
    }
    let mut bytes = snapshot_header(fact_hash, seq).into_bytes();
    bytes.extend(replay.snapshot());
    if let Err(e) = cartograph_core::paths::write_private(&ws.snapshot_path(), &bytes) {
        tracing::warn!(error = %e, "can't write the replay snapshot; the next sync resolves the tree again");
    }
}

/// The synthetic step that closed a removed folder's files ([`Rows::removal`]): it is not a
/// commit of the repository, so it never becomes the repository's indexed commit.
fn is_removal(step: &CommitStep) -> bool {
    step.author == "cartograph" && step.sha.as_bytes()[16..] == [0xff; 4]
}

/// A removed repository's steps after `floor`, oldest first, rebuilt from its rows: its commits,
/// and the file versions that opened or closed at each. Paths are namespaced as stored; those the
/// workspace now excludes are left out. The facts of every blob they add go into `facts`.
fn removed_history(
    conn: &rusqlite::Connection,
    repo: i64,
    floor: i64,
    exclude: &[String],
    facts: &mut Facts,
) -> rusqlite::Result<Vec<CommitStep>> {
    use cartograph_git::Change as GitChange;
    use std::collections::BTreeMap;
    let mut at: BTreeMap<i64, BTreeMap<String, GitChange>> = BTreeMap::new();
    let mut st = conn.prepare(
        "SELECT fv.from_seq, fv.to_seq, fv.blob,
                (SELECT path FROM file_path p WHERE p.file = fv.file ORDER BY from_seq LIMIT 1)
         FROM file_version fv JOIN file f ON f.id = fv.file
         WHERE f.repo = ?1 AND (fv.from_seq > ?2 OR fv.to_seq > ?2)",
    )?;
    let mut rows = st.query(params![repo, floor])?;
    while let Some(r) = rows.next()? {
        let (from, to): (i64, Option<i64>) = (r.get(0)?, r.get(1)?);
        let (Some(blob), Some(path)) = (
            oid(r.get_ref(2)?.as_blob()?),
            r.get::<_, Option<String>>(3)?,
        ) else {
            continue;
        };
        let inner = path.split_once('/').map_or(path.as_str(), |(_, p)| p);
        if crate::timeline::excluded(exclude, inner) {
            continue;
        }
        if from > floor {
            change(&mut at, from, &path).new = Some(blob);
            if let Some((_, kind)) = extractor_for(&path)
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
        if let Some(to) = to.filter(|t| *t > floor) {
            change(&mut at, to, &path).old = Some(blob);
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
        steps.push(CommitStep {
            sha,
            time: r.get(2)?,
            corrected_time: r.get(3)?,
            author: r.get(4)?,
            summary: r.get(5)?,
            changes: at
                .remove(&seq)
                .map(|m| m.into_values().collect())
                .unwrap_or_default(),
        });
    }
    Ok(steps)
}

fn change<'a>(
    at: &'a mut std::collections::BTreeMap<
        i64,
        std::collections::BTreeMap<String, cartograph_git::Change>,
    >,
    seq: i64,
    path: &str,
) -> &'a mut cartograph_git::Change {
    at.entry(seq)
        .or_default()
        .entry(path.to_owned())
        .or_insert_with(|| cartograph_git::Change {
            path: path.to_owned(),
            old: None,
            new: None,
        })
}

/// Removes every step after `floor`: rows opened after it go, rows closed after it reopen.
fn truncate(conn: &rusqlite::Connection, floor: i64) -> rusqlite::Result<()> {
    for t in ["interval", "file_version", "file_path"] {
        conn.execute(&format!("DELETE FROM {t} WHERE from_seq > ?1"), [floor])?;
        conn.execute(
            &format!("UPDATE {t} SET to_seq = NULL WHERE to_seq > ?1"),
            [floor],
        )?;
    }
    conn.execute("DELETE FROM commits WHERE seq > ?1", [floor])?;
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

/// The rows a round writes. Paths map to their lineage across runs, read back from `file_path`.
struct Rows<'c> {
    conn: &'c rusqlite::Connection,
    /// Path -> (file id, node id, open blob).
    files: HashMap<String, (i64, i64, Option<ObjectId>)>,
    links: HashMap<Link, i64>,
    /// Links open at or before this step have their open row written.
    written: i64,
    /// Repo id -> its last step written: (seq, sha).
    last: HashMap<i64, (i64, ObjectId)>,
}

impl<'c> Rows<'c> {
    fn load(conn: &'c rusqlite::Connection, floor: i64) -> rusqlite::Result<Self> {
        let mut files = HashMap::new();
        let mut st = conn.prepare(
            "SELECT fp.path, fp.file, n.id, fp.to_seq IS NULL FROM file_path fp
             JOIN node n ON n.file = fp.file AND n.kind = 'file' ORDER BY fp.from_seq",
        )?;
        let mut live = Vec::new();
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let path: String = r.get(0)?;
            if r.get::<_, bool>(3)? {
                live.push(path.clone());
            }
            files.insert(path, (r.get(1)?, r.get(2)?, None));
        }
        let mut st = conn.prepare(
            "SELECT fp.path, fv.blob FROM file_version fv
             JOIN file_path fp ON fp.file = fv.file AND fp.to_seq IS NULL
             WHERE fv.to_seq IS NULL",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let path: String = r.get(0)?;
            if let Some(f) = files.get_mut(&path) {
                f.2 = oid(r.get_ref(1)?.as_blob()?);
            }
        }
        Ok(Self {
            conn,
            files,
            links: HashMap::new(),
            written: floor,
            last: HashMap::new(),
        })
    }

    fn commit(&mut self, seq: i64, repo: i64, s: &CommitStep) -> rusqlite::Result<()> {
        self.last.insert(repo, (seq, s.sha));
        self.commit_only(seq, repo, s)
    }

    /// The commit row alone, without making it the repository's indexed commit.
    fn commit_only(&mut self, seq: i64, repo: i64, s: &CommitStep) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                seq,
                repo,
                s.sha.as_bytes(),
                s.time,
                s.corrected_time,
                s.author,
                s.summary
            ])?;
        Ok(())
    }

    /// The synthetic step of a folder removed from the workspace. Its sha is 20 bytes no git object
    /// can have by accident: a marker and the step.
    fn removal(&mut self, seq: i64, repo: i64, name: &str, corrected: i64) -> rusqlite::Result<()> {
        let mut sha = [0xffu8; 20];
        sha[..8].copy_from_slice(&seq.to_be_bytes());
        sha[8..16].copy_from_slice(&repo.to_be_bytes());
        self.conn.execute(
            "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
             VALUES (?1, ?2, ?3, ?4, ?4, 'cartograph', ?5)",
            params![
                seq,
                repo,
                &sha[..],
                corrected,
                format!("{name}: removed from workspace")
            ],
        )?;
        Ok(())
    }

    /// One lineage per path until IDX-18 detects renames: a path that comes back keeps its file.
    fn file_change(
        &mut self,
        repo: i64,
        seq: i64,
        path: &str,
        new: Option<ObjectId>,
    ) -> rusqlite::Result<()> {
        let (file, _, old) = match self.files.get(path) {
            Some(f) => *f,
            None => {
                self.conn
                    .prepare_cached("INSERT INTO file (repo) VALUES (?1)")?
                    .execute([repo])?;
                let file = self.conn.last_insert_rowid();
                self.conn
                    .prepare_cached("INSERT INTO node (kind, file) VALUES ('file', ?1)")?
                    .execute([file])?;
                let node = self.conn.last_insert_rowid();
                self.files.insert(path.to_owned(), (file, node, None));
                (file, node, None)
            }
        };
        if old.is_some() {
            self.conn
                .prepare_cached(
                    "UPDATE file_version SET to_seq = ?2 WHERE file = ?1 AND to_seq IS NULL",
                )?
                .execute(params![file, seq])?;
            if new.is_none() {
                self.conn
                    .prepare_cached(
                        "UPDATE file_path SET to_seq = ?2 WHERE file = ?1 AND to_seq IS NULL",
                    )?
                    .execute(params![file, seq])?;
            }
        }
        if let Some(n) = new {
            if old.is_none() {
                self.conn
                    .prepare_cached(
                        "INSERT INTO file_path (file, path, from_seq) VALUES (?1, ?2, ?3)",
                    )?
                    .execute(params![file, path, seq])?;
            }
            self.conn
                .prepare_cached(
                    "INSERT INTO file_version (file, blob, from_seq) VALUES (?1, ?2, ?3)",
                )?
                .execute(params![file, n.as_bytes(), seq])?;
        }
        self.files.get_mut(path).expect("inserted").2 = new;
        Ok(())
    }

    fn link_id(&mut self, link: &Link) -> rusqlite::Result<i64> {
        if let Some(id) = self.links.get(link) {
            return Ok(*id);
        }
        let node = |p: &str| self.files.get(p).expect("a link joins two indexed files").1;
        let (src, dst) = (node(&link.1), node(&link.2));
        self.conn
            .prepare_cached(
                "INSERT INTO link (kind, src, dst) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            )?
            .execute(params![link.0.as_str(), src, dst])?;
        let id = self
            .conn
            .prepare_cached("SELECT id FROM link WHERE kind = ?1 AND src = ?2 AND dst = ?3")?
            .query_row(params![link.0.as_str(), src, dst], |r| r.get(0))?;
        self.links.insert(link.clone(), id);
        Ok(id)
    }

    fn interval(&mut self, link: &Link, from: i64, to: Option<i64>) -> rusqlite::Result<()> {
        let id = self.link_id(link)?;
        self.conn
            .prepare_cached("INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)")?
            .execute(params![id, from, to])?;
        Ok(())
    }

    /// A closed interval. One that opened at or before the last flush has its open row: it gets
    /// its end.
    fn close(&mut self, i: &Interval) -> rusqlite::Result<()> {
        if i.from as i64 > self.written {
            return self.interval(&i.link, i.from as i64, Some(i.to as i64));
        }
        let id = self.link_id(&i.link)?;
        self.conn
            .prepare_cached("UPDATE interval SET to_seq = ?2 WHERE link = ?1 AND to_seq IS NULL")?
            .execute(params![id, i.to as i64])?;
        Ok(())
    }

    /// Before a batch commits: the open rows of links that opened since the last flush, each
    /// repository's indexed commit and the watermark, so the batch is a complete index.
    fn flush(&mut self, replay: &Replay, seq: i64) -> rusqlite::Result<()> {
        let mut open: Vec<(&Link, u64)> = replay
            .open()
            .filter(|(_, from)| *from as i64 > self.written)
            .collect();
        open.sort();
        for (l, from) in open {
            self.interval(l, from as i64, None)?;
        }
        self.written = seq;
        for (repo, (at, sha)) in self.last.drain() {
            self.conn
                .prepare_cached(
                    "UPDATE repo SET indexed_commit = ?2, indexed_seq = ?3, head = ?2 WHERE id = ?1",
                )?
                .execute(params![repo, sha.as_bytes(), at])?;
        }
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('watermark_seq', ?1)",
            )?
            .execute([seq])?;
        Ok(())
    }

    /// Facts of each blob, once.
    fn facts<'f>(
        &mut self,
        facts: impl Iterator<Item = (&'f (ObjectId, &'static str), &'f Arc<BlobFacts>)>,
    ) -> rusqlite::Result<()> {
        let mut kinds: HashMap<&str, i64> = HashMap::new();
        let mut rows: Vec<_> = facts.collect();
        rows.sort_by_key(|((o, k), _)| (*o, *k));
        for ((oid, kind), f) in rows {
            let ex = match kinds.get(kind) {
                Some(id) => *id,
                None => {
                    let version = extractor_of(kind).version();
                    self.conn.execute(
                        "INSERT INTO extractor (name, version) VALUES (?1, ?2)
                         ON CONFLICT DO NOTHING",
                        params![kind, version],
                    )?;
                    let id = self.conn.query_row(
                        "SELECT id FROM extractor WHERE name = ?1 AND version = ?2",
                        params![kind, version],
                        |r| r.get(0),
                    )?;
                    kinds.insert(kind, id);
                    id
                }
            };
            self.conn
                .prepare_cached(
                    "INSERT INTO fact (blob, extractor, payload) VALUES (?1, ?2, ?3)
                     ON CONFLICT DO NOTHING",
                )?
                .execute(params![oid.as_bytes(), ex, f.encode()])?;
        }
        Ok(())
    }
}
