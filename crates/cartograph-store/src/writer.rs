//! The single writer (IDX-13, B2, D2).
//!
//! Only the process holding `<dir>/index.lock` writes the index. The lock is an OS lock
//! (`flock` on Unix, `LockFileEx` on Windows) that the kernel releases when the process dies, so it
//! never goes stale; the pid in the file is for display only. A request that finds the lock held
//! sets `<dir>/dirty` and returns at once, and the holder runs one more round before it lets go
//! ([`run_exclusive`]). Writes go in batches ([`Batches`]): each commit bumps `meta.generation`,
//! which readers use to notice new data (SRV-02).
//!
//! The lock, dirty, cancel and progress files live in the user-only workspace folder, so another
//! local user can't cancel or block a sync.

use std::cell::Cell;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

pub const LOCK_FILE: &str = "index.lock";
pub const DIRTY_FILE: &str = "dirty";
pub const CANCEL_FILE: &str = "cancel";
pub const PROGRESS_FILE: &str = "progress.json";

/// A batch is committed after this many timeline steps (proposed, IDX-13)…
pub const BATCH_STEPS: usize = 1000;
/// …or after this long, whichever comes first. Also bounds how long a cancel waits.
pub const BATCH_AGE: Duration = Duration::from_secs(2);
/// Progress is rewritten at most four times a second (§4.8).
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Proof that this process is the one writer of the index in a folder.
#[derive(Debug)]
pub struct WriterLock {
    // Held for its OS lock, which closing the file (or the process dying) releases.
    _file: File,
    dir: PathBuf,
    progress_at: Cell<Option<Instant>>,
}

fn private(opts: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(opts, 0o600);
    opts
}

impl WriterLock {
    /// Takes the lock without waiting. `Ok(None)` when another process (or another handle in this
    /// one) holds it.
    pub fn try_acquire(dir: &Path) -> io::Result<Option<Self>> {
        let mut file = private(OpenOptions::new().read(true).write(true).create(true))
            .truncate(false)
            .open(dir.join(LOCK_FILE))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        // A cancel asked for while nobody was writing doesn't cancel the next run.
        remove_if_present(&dir.join(CANCEL_FILE))?;
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        file.set_len(0)?;
        writeln!(file, "pid {}\nsince {since}", std::process::id())?;
        Ok(Some(Self {
            _file: file,
            dir: dir.to_owned(),
            progress_at: Cell::new(None),
        }))
    }

    /// The folder the lock guards.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Clears the dirty flag; true if it was set.
    pub fn take_dirty(&self) -> io::Result<bool> {
        remove_if_present(&self.dir.join(DIRTY_FILE))
    }

    /// True once someone asked this run to stop (E5): [`request_cancel`] or the cancel file. The
    /// writer finishes and commits the current batch, then stops.
    ///
    /// ponytail: SIGINT and SIGTERM still end the process at once (std has no signal API and the
    /// workspace forbids unsafe). That is a crash, which the WAL makes safe: the next run finds
    /// state `syncing`, marks it failed and resumes. Wire the signals to the cancel file with
    /// signal-hook if an abrupt stop shows up as a problem.
    pub fn cancel_requested(&self) -> bool {
        self.dir.join(CANCEL_FILE).exists()
    }

    /// Rewrites `progress.json` atomically, at most four times a second unless `force`. It holds
    /// counts and repo names only, no paths or code.
    pub fn progress(&self, p: &Progress<'_>, force: bool) -> io::Result<()> {
        let now = Instant::now();
        if !force
            && self
                .progress_at
                .get()
                .is_some_and(|t| now.duration_since(t) < PROGRESS_EVERY)
        {
            return Ok(());
        }
        self.progress_at.set(Some(now));
        let body = serde_json::json!({
            "pid": std::process::id(),
            "phase": p.phase,
            "done": p.done,
            "total": p.total,
            "repo": p.repo,
        });
        let tmp = self.dir.join(format!(".{PROGRESS_FILE}.tmp"));
        let mut f =
            private(OpenOptions::new().write(true).create(true).truncate(true)).open(&tmp)?;
        f.write_all(body.to_string().as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, self.dir.join(PROGRESS_FILE))
    }
}

/// What `progress.json` says while a run is going.
#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    pub phase: &'a str,
    pub done: u64,
    pub total: u64,
    pub repo: Option<&'a str>,
}

/// Asks the running writer of `dir`, if any, to stop after its current batch.
pub fn request_cancel(dir: &Path) -> io::Result<()> {
    touch(&dir.join(CANCEL_FILE))
}

/// Sets the dirty flag: the running writer runs one more round before it finishes (D2).
pub fn mark_dirty(dir: &Path) -> io::Result<()> {
    touch(&dir.join(DIRTY_FILE))
}

fn touch(p: &Path) -> io::Result<()> {
    private(OpenOptions::new().write(true).create(true))
        .truncate(false)
        .open(p)
        .map(drop)
}

fn remove_if_present(p: &Path) -> io::Result<bool> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// What [`run_exclusive`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Exclusive<T> {
    /// Another process is writing. The dirty flag is set, so it runs one more round; the caller
    /// exits 0 (D2).
    Busy,
    /// This process was the writer: the last round's result and how many rounds ran.
    Ran { last: T, rounds: u32 },
}

/// The D2 protocol. Takes the lock, or sets the dirty flag and returns [`Exclusive::Busy`] at once.
/// The holder runs `round` until no request came in during the last one, until a cancel, or until
/// `max_rounds` rounds have run; then a dirty flag that is still set stays for the next request.
///
/// A request can land between the holder's last dirty check and its release; it found the lock
/// held, so after releasing, the holder looks again and takes the lock back if it can.
pub fn run_exclusive<T, E: From<io::Error>>(
    dir: &Path,
    max_rounds: Option<u32>,
    mut round: impl FnMut(&WriterLock) -> Result<T, E>,
) -> Result<Exclusive<T>, E> {
    // The flag goes up before the lock is tried, as sync() does: set after a failed try, it could
    // land after the holder's last look and stay set with no writer. Taking the lock, this process
    // clears its own flag at the start of its first round.
    mark_dirty(dir)?;
    let Some(mut lock) = WriterLock::try_acquire(dir)? else {
        return Ok(Exclusive::Busy);
    };
    let mut rounds = 0;
    loop {
        // This round covers every request made before it started.
        lock.take_dirty()?;
        let last = round(&lock)?;
        rounds += 1;
        if lock.cancel_requested() || max_rounds.is_some_and(|m| rounds >= m) {
            return Ok(Exclusive::Ran { last, rounds });
        }
        if lock.dir.join(DIRTY_FILE).exists() {
            continue;
        }
        drop(lock);
        if !dir.join(DIRTY_FILE).exists() {
            return Ok(Exclusive::Ran { last, rounds });
        }
        match WriterLock::try_acquire(dir)? {
            Some(l) => lock = l,
            // Whoever holds it now will see the flag.
            None => return Ok(Exclusive::Ran { last, rounds }),
        }
    }
}

/// Bumps `meta.generation`, in the caller's transaction.
pub fn bump_generation(conn: &Connection) -> rusqlite::Result<()> {
    conn.prepare_cached("UPDATE meta SET value = value + 1 WHERE key = 'generation'")?
        .execute([])
        .map(drop)
}

/// Commits a run in batches: one transaction per [`BATCH_STEPS`] steps or [`BATCH_AGE`], whichever
/// comes first. Every commit bumps `meta.generation`. Write `repo.indexed_commit`,
/// `repo.indexed_seq` and `meta.watermark_seq` before [`Batches::commit`], so they land in the same
/// transaction as the rows they cover and a crash leaves a complete, earlier index.
#[derive(Debug)]
pub struct Batches {
    since: Instant,
    steps: usize,
}

impl Batches {
    /// Opens the first batch.
    pub fn begin(conn: &Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self {
            since: Instant::now(),
            steps: 0,
        })
    }

    /// Counts one step; true when the batch is due.
    pub fn step(&mut self) -> bool {
        self.steps += 1;
        self.steps >= BATCH_STEPS || self.since.elapsed() >= BATCH_AGE
    }

    /// Commits the batch with a new generation and opens the next.
    pub fn commit(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        bump_generation(conn)?;
        conn.execute_batch("COMMIT; BEGIN IMMEDIATE")?;
        self.since = Instant::now();
        self.steps = 0;
        Ok(())
    }

    /// Commits the last batch, then truncates the WAL and refreshes the planner's statistics.
    pub fn finish(self, conn: &Connection) -> rusqlite::Result<()> {
        bump_generation(conn)?;
        conn.execute_batch("COMMIT")?;
        checkpoint(conn)?;
        conn.execute_batch("PRAGMA optimize")
    }
}

/// Truncates the WAL. A reader that holds an old snapshot blocks that; then a passive checkpoint
/// copies what it can and the WAL is truncated by a later run, rather than waiting forever.
pub fn checkpoint(conn: &Connection) -> rusqlite::Result<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        tracing::warn!("a reader blocks truncating the WAL; checkpointing passively");
        conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| r.get::<_, i64>(0))?;
    }
    Ok(())
}

/// Call after taking the lock, before writing. A run that died leaves `meta.state` at `syncing`
/// (or `building`) with nobody holding the lock: it becomes `failed`, with the reason in
/// `meta.last_error`. Returns the state that was found.
pub fn recover(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let state: Option<String> = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = 'state'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if matches!(state.as_deref(), Some("syncing" | "building")) {
        tracing::warn!("the last run stopped part way through; resuming");
        conn.execute_batch(
            "UPDATE meta SET value = 'failed' WHERE key = 'state';
             INSERT OR REPLACE INTO meta (key, value)
               VALUES ('last_error', 'the last run stopped part way through');",
        )?;
    }
    Ok(state)
}
