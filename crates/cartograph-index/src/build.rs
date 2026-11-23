//! The vertical slice (IDX-07): one repository, walked, extracted and replayed into one new index
//! file, where every import link carries the commit that opened it and, once removed, the commit
//! that closed it.
//!
//! ponytail: walk, extract and replay run one after another, not as a pipeline over a bounded
//! channel; on remotion the walk is under 2 s and the steps fit in memory. No registry, manifest or
//! lock yet (IDX-11, IDX-13).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use cartograph_core::Explain;
use cartograph_facts::replay::Change;
use cartograph_facts::{FactCache, Interval, Link, Replay};
use cartograph_git::{CommitStep, GitError, ObjectId, Repo};
use cartograph_parse::{extractor_for, tracked};
use cartograph_store::hash::store_fact_hash;
use cartograph_store::rusqlite::{self, params};
use cartograph_store::writer::Batches;
use cartograph_store::{Index, StoreError};

use crate::extract::{ExtractStats, Facts, extract, extractor_of};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("can't write the index")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0} has no commits")]
    Empty(String),
}

impl Explain for BuildError {
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
            Self::Empty(_) => "there is no history to index".into(),
        }
    }
    fn action(&self) -> String {
        match self {
            Self::Git(e) => e.action(),
            Self::Store(e) => e.action(),
            Self::Sqlite(_) => "check the free disk space".into(),
            Self::Empty(_) => "make a first commit".into(),
        }
    }
}

/// What a build did, with its timings in milliseconds.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub steps: usize,
    pub change_events: usize,
    pub extract: ExtractStats,
    pub resolutions: u64,
    pub links: usize,
    pub intervals: usize,
    pub open_at_head: usize,
    /// Live references at head: (links, external, builtin, unresolved).
    pub outcomes: (usize, usize, usize, usize),
    pub walk_ms: u128,
    pub extract_ms: u128,
    pub replay_ms: u128,
    pub total_ms: u128,
}

/// The worker count for extraction: min(cores, 8), until IDX-02 settles it.
pub fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get().min(8))
}

/// Walks the first-parent history up to `to` and extracts the facts of every blob it adds.
pub fn walk_and_extract(
    repo: &Repo,
    to: ObjectId,
    cache: &mut FactCache,
    workers: usize,
) -> Result<(Vec<CommitStep>, Facts, ExtractStats, u128, u128), BuildError> {
    let t = Instant::now();
    let steps: Vec<CommitStep> = repo
        .walk_first_parent(None, to, tracked)?
        .collect::<Result<_, _>>()?;
    let walk_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("a thread pool");
    let mut facts = Facts::new();
    let items = steps
        .iter()
        .flat_map(|s| &s.changes)
        .filter_map(|c| Some((c.new?, extractor_for(&c.path)?.1)));
    let st = extract(repo, items, cache, &pool, &mut facts)?;
    Ok((steps, facts, st, walk_ms, t.elapsed().as_millis()))
}

/// A step's changes with their facts, as the replay takes them.
pub fn changes_of(step: &CommitStep, facts: &Facts) -> Vec<Change> {
    step.changes
        .iter()
        .map(|c| {
            let f = c.new.and_then(|o| {
                let kind = extractor_for(&c.path)?.1;
                facts.get(&(o, kind)).map(Arc::clone)
            });
            (c.path.clone(), f)
        })
        .collect()
}

/// Indexes the repository at `repo_path`, up to `to` (default HEAD), into a new file at `out`,
/// which must not exist.
pub fn build_single(
    repo_path: &Path,
    to: Option<&str>,
    out: &Path,
    cache: &mut FactCache,
    workers: usize,
) -> Result<Summary, BuildError> {
    let t0 = Instant::now();
    let repo = Repo::open(repo_path)?;
    let head = repo
        .resolve(to.unwrap_or("HEAD"))
        .map_err(|_| BuildError::Empty(repo_path.display().to_string()))?;
    let (steps, facts, extract, walk_ms, extract_ms) =
        walk_and_extract(&repo, head, cache, workers)?;
    let ix = Index::create(out)?;
    let t = Instant::now();
    let mut w = Writer::new(&ix.conn)?;
    let repo_name = repo_path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "repo".into());
    ix.conn.execute(
        "INSERT INTO repo (id, name, path, head, shallow) VALUES (1, ?1, ?2, ?3, ?4)",
        params![
            repo_name,
            repo_path.display().to_string(),
            head.as_bytes(),
            repo.is_shallow()
        ],
    )?;
    ix.conn.execute(
        "INSERT INTO run (id, started_at, writer_version, head) VALUES (1, unixepoch(), ?1, ?2)",
        params![env!("CARGO_PKG_VERSION"), head.as_bytes()],
    )?;
    let mut replay = Replay::default();
    let mut s = Summary {
        steps: steps.len(),
        extract,
        walk_ms,
        extract_ms,
        ..Default::default()
    };
    // Every committed batch bumps meta.generation (IDX-13). A build killed part way leaves a file
    // with state `building` that passes quick_check; it is never renamed into place (IDX-16).
    let mut batches = Batches::begin(&ix.conn)?;
    for (seq, step) in steps.iter().enumerate() {
        let seq = seq as u64;
        s.change_events += step.changes.len();
        w.commit(seq, step)?;
        replay.apply(&changes_of(step, &facts), seq);
        for c in &step.changes {
            w.file_change(seq, &c.path, c.new)?;
        }
        for i in replay.closed.drain(..) {
            w.interval(&i.link, i.from, Some(i.to))?;
            s.intervals += 1;
        }
        if batches.step() {
            batches.commit(&ix.conn)?;
        }
    }
    let mut open: Vec<(&Link, u64)> = replay.open().collect();
    open.sort();
    for (l, from) in &open {
        w.interval(l, *from, None)?;
    }
    s.open_at_head = open.len();
    s.intervals += open.len();
    w.facts(&facts)?;
    s.resolutions = replay.stats.resolutions;
    s.links = w.links.len();
    s.outcomes = replay.outcomes();
    s.replay_ms = t.elapsed().as_millis();
    s.total_ms = t0.elapsed().as_millis();
    ix.conn.execute(
        "UPDATE run SET finished_at = unixepoch(), stats = ?1 WHERE id = 1",
        [format!("{s:?}")],
    )?;
    let last = steps.len().saturating_sub(1) as i64;
    ix.conn.execute(
        "UPDATE repo SET indexed_commit = head, indexed_seq = ?1 WHERE id = 1",
        [last],
    )?;
    ix.conn.execute_batch(&format!(
        "UPDATE meta SET value = 'complete' WHERE key = 'state';
         INSERT OR REPLACE INTO meta (key, value) VALUES ('watermark_seq', {last});"
    ))?;
    store_fact_hash(&ix.conn)?;
    batches.finish(&ix.conn)?;
    Ok(s)
}

/// Rows for one build. Intervals are written when they close, and the open ones at the end.
struct Writer<'c> {
    conn: &'c rusqlite::Connection,
    /// Path -> (file id, node id, open blob).
    files: HashMap<String, (i64, i64, Option<ObjectId>)>,
    links: HashMap<Link, i64>,
}

impl<'c> Writer<'c> {
    fn new(conn: &'c rusqlite::Connection) -> rusqlite::Result<Self> {
        Ok(Self {
            conn,
            files: HashMap::new(),
            links: HashMap::new(),
        })
    }

    fn commit(&mut self, seq: u64, s: &CommitStep) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                seq as i64,
                s.sha.as_bytes(),
                s.time,
                s.corrected_time,
                s.author,
                s.summary
            ])?;
        Ok(())
    }

    /// One lineage per path until IDX-18 detects renames: a path that comes back keeps its file.
    fn file_change(&mut self, seq: u64, path: &str, new: Option<ObjectId>) -> rusqlite::Result<()> {
        let seq = seq as i64;
        let (file, _, old) = match self.files.get(path) {
            Some(f) => *f,
            None => {
                self.conn
                    .prepare_cached("INSERT INTO file (repo) VALUES (1)")?
                    .execute([])?;
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

    fn interval(&mut self, link: &Link, from: u64, to: Option<u64>) -> rusqlite::Result<()> {
        let id = match self.links.get(link) {
            Some(id) => *id,
            None => {
                let node = |p: &str| self.files.get(p).expect("a link joins two indexed files").1;
                self.conn
                    .prepare_cached("INSERT INTO link (kind, src, dst) VALUES (?1, ?2, ?3)")?
                    .execute(params![link.0.as_str(), node(&link.1), node(&link.2)])?;
                let id = self.conn.last_insert_rowid();
                self.links.insert(link.clone(), id);
                id
            }
        };
        self.conn
            .prepare_cached("INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)")?
            .execute(params![id, from as i64, to.map(|t| t as i64)])?;
        Ok(())
    }

    fn facts(&mut self, facts: &Facts) -> rusqlite::Result<()> {
        let mut kinds: HashMap<&str, i64> = HashMap::new();
        let mut rows: Vec<_> = facts.iter().collect();
        rows.sort_by_key(|((o, k), _)| (*o, *k));
        for ((oid, kind), f) in rows {
            let ex = match kinds.get(kind) {
                Some(id) => *id,
                None => {
                    self.conn.execute(
                        "INSERT INTO extractor (name, version) VALUES (?1, ?2)",
                        params![kind, extractor_of(kind).version()],
                    )?;
                    let id = self.conn.last_insert_rowid();
                    kinds.insert(kind, id);
                    id
                }
            };
            self.conn
                .prepare_cached("INSERT INTO fact (blob, extractor, payload) VALUES (?1, ?2, ?3)")?
                .execute(params![oid.as_bytes(), ex, f.encode()])?;
        }
        Ok(())
    }
}

/// The links live at `seq`, read back from an index: `(kind, from path, to path)`.
pub fn links_at(
    conn: &rusqlite::Connection,
    seq: u64,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut st = conn.prepare_cached(
        "SELECT l.kind, ps.path, pd.path
         FROM interval i JOIN link l ON l.id = i.link
         JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
         JOIN file_path ps ON ps.file = ns.file AND ps.from_seq <= ?1 AND (ps.to_seq IS NULL OR ps.to_seq > ?1)
         JOIN file_path pd ON pd.file = nd.file AND pd.from_seq <= ?1 AND (pd.to_seq IS NULL OR pd.to_seq > ?1)
         WHERE i.from_seq <= ?1 AND (i.to_seq IS NULL OR i.to_seq > ?1)",
    )?;
    let rows = st.query_map([seq as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut v: Vec<_> = rows.collect::<Result<_, _>>()?;
    v.sort();
    Ok(v)
}

/// The first step of a commit, by sha (a commit can sit in two repos).
pub fn seq_of(conn: &rusqlite::Connection, sha: ObjectId) -> rusqlite::Result<Option<u64>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT seq FROM commits WHERE sha = ?1 ORDER BY seq LIMIT 1",
        [sha.as_bytes()],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|o| o.map(|s| s as u64))
}

/// Every interval in `replay`, open ones running to `end`.
pub fn all_intervals(replay: &Replay, closed: &[Interval], end: u64) -> Vec<Interval> {
    let mut v = closed.to_vec();
    v.extend(replay.open().map(|(l, s)| Interval {
        link: l.clone(),
        from: s,
        to: end,
    }));
    v.sort();
    v
}
