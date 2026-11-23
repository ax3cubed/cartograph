//! The reader pool and snapshots (SRV-02).
//!
//! Every connection is read-only (`SQLITE_OPEN_READONLY`, `query_only`), opened through the store's
//! format gate. A [`Snapshot`] is one read transaction: everything read through it comes from one
//! generation. No snapshot outlives its [`Limits::deadline`], so a reader never holds the WAL open
//! for long (B2).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cartograph_store::rusqlite::{Connection, OptionalExtension};
use cartograph_store::{Index, State};

use crate::QueryError;
use crate::adjacency::Adjacency;

/// Default deadline for one request's snapshot.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
/// No snapshot lives longer than this, whatever the caller asks.
pub const HARD_DEADLINE: Duration = Duration::from_secs(30);
/// SQLite runs the progress handler every this many VM steps; a few microseconds apart.
const PROGRESS_OPS: i32 = 1000;

/// What one request may spend.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Rows a single answer may hold, all tables together.
    pub max_rows: usize,
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // The graph's response cap: 100k nodes and 400k links (SRV-04).
            max_rows: 500_000,
            deadline: DEFAULT_DEADLINE,
        }
    }
}

// ponytail: no max_bytes yet. Every answer so far is rows of ids and short paths, so max_rows
// bounds the bytes; add a byte budget with the first route that returns text (SRV-08 /source).

/// One workspace's index, shared by every request.
pub struct Store {
    path: PathBuf,
    size: usize,
    pool: Mutex<Pool>,
    freed: Condvar,
    /// The graph of the last generation read, built on first use (SRV-11).
    graph: Mutex<Option<Arc<Adjacency>>>,
    /// The last few graph slices, by generation and request (SRV-04's cached mark).
    slices: Mutex<Vec<(SliceKey, Arc<cartograph_api::types::GraphSlice>)>>,
}

type SliceKey = (u64, u64, String);
/// Slices kept: a map open at a few dates and levels.
const SLICES: usize = 8;

struct Pool {
    idle: Vec<Pooled>,
    /// Connections that exist, idle or lent out.
    open: usize,
    /// Bumped when the file is replaced; connections from an older epoch are closed on return.
    epoch: u64,
    file: Option<FileId>,
    state: State,
}

struct Pooled {
    conn: Connection,
    epoch: u64,
}

/// What identifies the file on disk, so a rebuild renamed into place is noticed (B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId(u64, u64);

fn file_id(path: &Path) -> Option<FileId> {
    let m = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileId(m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        // ponytail: Windows has no inode in std; size and mtime catch a rebuild renamed into place.
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        Some(FileId(m.len(), mtime))
    }
}

/// A workspace id names a folder under the data folder, so only `[A-Za-z0-9_-]` is accepted.
pub fn valid_workspace_id(ws: &str) -> bool {
    (1..=128).contains(&ws.len())
        && ws
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Store {
    /// The index of workspace `ws` under the data folder.
    pub fn open(data_dir: &Path, ws: &str) -> Result<Self, QueryError> {
        if !valid_workspace_id(ws) {
            return Err(QueryError::NotFound {
                what: "no such workspace".into(),
                action: "list workspaces with GET /api/v1/workspaces".into(),
            });
        }
        Self::open_path(
            &cartograph_core::paths::workspace_dir(data_dir, ws).join("index.cartograph"),
        )
    }

    /// Opens the first connection now, so a missing, damaged or too-new file fails here.
    pub fn open_path(path: &Path) -> Result<Self, QueryError> {
        let size = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let file = file_id(path);
        let ix = Index::open_read(path)?;
        let state = ix.state;
        let store = Self {
            path: path.to_owned(),
            size,
            pool: Mutex::new(Pool {
                idle: Vec::new(),
                open: 1,
                epoch: 0,
                file,
                state,
            }),
            freed: Condvar::new(),
            graph: Mutex::new(None),
            slices: Mutex::new(Vec::new()),
        };
        store.lock().idle.push(Pooled {
            conn: tune(ix.conn)?,
            epoch: 0,
        });
        Ok(store)
    }

    /// What the open rules said about the file.
    pub fn state(&self) -> State {
        self.lock().state
    }

    /// Pool size: `min(8, cores)`.
    pub fn size(&self) -> usize {
        self.size
    }

    fn lock(&self) -> MutexGuard<'_, Pool> {
        // A panic while holding the lock leaves the pool consistent: every field is set in one step.
        self.pool.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Begins one read transaction. Waits for a free connection until the deadline.
    pub fn snapshot(&self, limits: Limits) -> Result<Snapshot<'_>, QueryError> {
        let deadline = Instant::now() + limits.deadline.min(HARD_DEADLINE);
        let pooled = self.acquire(deadline)?;
        let mut snap = Snapshot {
            store: self,
            conn: Some(pooled),
            generation: 0,
            fact_hash: None,
            limits,
            deadline,
        };
        snap.begin()?;
        Ok(snap)
    }

    fn acquire(&self, deadline: Instant) -> Result<Pooled, QueryError> {
        let mut pool = self.lock();
        let now = file_id(&self.path);
        if now.is_some() && now != pool.file {
            tracing::info!("index file replaced; reopening the reader pool");
            pool.epoch += 1;
            pool.file = now;
            pool.open -= pool.idle.len();
            pool.idle.clear();
        }
        loop {
            if let Some(p) = pool.idle.pop() {
                return Ok(p);
            }
            if pool.open < self.size {
                pool.open += 1;
                let epoch = pool.epoch;
                drop(pool);
                let opened = Index::open_read(&self.path)
                    .map_err(QueryError::from)
                    .and_then(|ix| {
                        let state = ix.state;
                        Ok((tune(ix.conn)?, state))
                    });
                let mut pool = self.lock();
                return match opened {
                    Ok((conn, state)) => {
                        pool.state = state;
                        Ok(Pooled { conn, epoch })
                    }
                    Err(e) => {
                        pool.open -= 1;
                        self.freed.notify_one();
                        Err(e)
                    }
                };
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(QueryError::Timeout);
            }
            pool = self
                .freed
                .wait_timeout(pool, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    fn release(&self, p: Pooled) {
        let mut pool = self.lock();
        if p.epoch == pool.epoch {
            pool.idle.push(p);
        } else {
            pool.open -= 1;
        }
        drop(pool);
        self.freed.notify_one();
    }
}

/// Per-connection settings beyond the store's hardening (which sets `query_only` and mmap).
fn tune(conn: Connection) -> Result<Connection, QueryError> {
    conn.execute_batch("PRAGMA cache_size = -16000;")?;
    // ponytail: rusqlite's statement cache holds 16 by default; enough for the handful of queries
    // each route runs. Raise it with set_prepared_statement_cache_capacity when routes multiply.
    Ok(conn)
}

/// One read transaction on one connection. Everything read through it comes from `generation`.
pub struct Snapshot<'s> {
    store: &'s Store,
    conn: Option<Pooled>,
    /// The index generation: bumped by every committed writer batch.
    pub generation: u64,
    /// `meta.fact_hash`, once the indexer writes it (B4).
    pub fact_hash: Option<String>,
    pub limits: Limits,
    deadline: Instant,
}

impl Snapshot<'_> {
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn.as_ref().expect("held until drop").conn
    }

    pub(crate) fn store_state(&self) -> State {
        self.store.state()
    }

    fn begin(&mut self) -> Result<(), QueryError> {
        let deadline = self.deadline;
        let p = self.conn.as_mut().expect("held until drop");
        p.conn
            .progress_handler(PROGRESS_OPS, Some(move || Instant::now() >= deadline))?;
        p.conn.execute_batch("BEGIN")?;
        // The first read starts the transaction; every later read sees the same generation.
        let meta = |k: &str| -> Result<Option<String>, QueryError> {
            Ok(self
                .conn()
                .prepare_cached("SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1")?
                .query_row([k], |r| r.get(0))
                .optional()?)
        };
        let generation = meta("generation")?;
        self.fact_hash = meta("fact_hash")?;
        self.generation = generation.and_then(|g| g.parse().ok()).unwrap_or(0);
        if self.store.state() == State::NeedsMigrate {
            return Err(QueryError::Unavailable {
                why: "an older Cartograph wrote this index".into(),
                action: "cartograph index --rebuild".into(),
            });
        }
        Ok(())
    }

    /// The in-memory graph of this snapshot's generation: the cached one, or built now and cached.
    /// Two requests racing on a new generation may both build it; the last one is kept.
    pub(crate) fn adjacency(&self) -> Result<Arc<Adjacency>, QueryError> {
        let epoch = self.conn.as_ref().expect("held until drop").epoch;
        let key = (epoch, self.generation);
        let cached = self
            .store
            .graph
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(g) = cached.filter(|g| g.key == key) {
            return Ok(g);
        }
        let g = Arc::new(Adjacency::build(self, key)?);
        *self.store.graph.lock().unwrap_or_else(|p| p.into_inner()) = Some(g.clone());
        Ok(g)
    }

    /// A slice this generation already built for the same request, if any.
    pub(crate) fn cached_slice(&self, key: &str) -> Option<Arc<cartograph_api::types::GraphSlice>> {
        let k = self.slice_key(key);
        self.store
            .slices
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|(x, _)| *x == k)
            .map(|(_, s)| s.clone())
    }

    pub(crate) fn keep_slice(&self, key: &str, slice: Arc<cartograph_api::types::GraphSlice>) {
        let k = self.slice_key(key);
        let mut all = self.store.slices.lock().unwrap_or_else(|p| p.into_inner());
        all.retain(|(x, _)| x.0 == k.0 && x.1 == k.1 && *x != k);
        if all.len() >= SLICES {
            all.remove(0);
        }
        all.push((k, slice));
    }

    fn slice_key(&self, key: &str) -> SliceKey {
        let epoch = self.conn.as_ref().expect("held until drop").epoch;
        (epoch, self.generation, key.to_owned())
    }

    /// Fails with `too_large` when an answer would pass the row budget.
    pub(crate) fn check_rows(&self, rows: usize, narrower: &str) -> Result<(), QueryError> {
        if rows > self.limits.max_rows {
            return Err(QueryError::TooLarge {
                why: format!(
                    "the answer has more than {} rows, the limit for one response",
                    self.limits.max_rows
                ),
                narrower: narrower.into(),
            });
        }
        Ok(())
    }
}

impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        let Some(mut p) = self.conn.take() else {
            return;
        };
        // A read transaction has nothing to commit; ending it lets the WAL be checkpointed.
        let ended = p.conn.execute_batch("ROLLBACK").is_ok() || p.conn.is_autocommit();
        let _ = p.conn.progress_handler(0, None::<fn() -> bool>);
        if ended {
            self.store.release(p);
        } else {
            // Never hand on a connection still inside a transaction.
            self.store.release(Pooled {
                epoch: u64::MAX,
                ..p
            });
        }
    }
}
