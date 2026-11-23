//! Rebuilds and migrations by atomic rename (IDX-16, B2, B3).
//!
//! Only a rebuild or a migration writes a new file. It is built beside the index as
//! `index.cartograph.new` (O_EXCL, 0600, same folder, so the rename is atomic), fsynced, renamed
//! over the index, and the folder fsynced. A reader holding the old file open keeps answering from
//! it, and sees the new one when it next opens (the query layer compares the inode). A crash before
//! the rename leaves the old index untouched and a `.new` file that the next run deletes.
//!
//! ponytail: no Windows path yet. Windows can't rename over an open file; the plan writes
//! `index.<n>.cartograph` and switches a pointer file there. Until then a rebuild on Windows fails
//! at the rename while a reader is open, and the old index stays as it was.

use std::io;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::writer::WriterLock;
use crate::{FORMAT_VERSION, Index, State, StoreError, USER_VERSION, harden, sidecar};

/// One migration step: the SQL that takes format `from` to `from + 1`. Each runs in one
/// transaction on a copy of the index.
pub type Step = (i64, &'static str);

/// The registered migrations. A version pair with no path through them means rebuild, since the
/// index is a cache (B3).
pub const MIGRATIONS: &[Step] = &[(
    1,
    "ALTER TABLE repo ADD COLUMN indexed_commit BLOB;
     ALTER TABLE repo ADD COLUMN indexed_seq INTEGER;
     ALTER TABLE repo ADD COLUMN removed INTEGER NOT NULL DEFAULT 0;
     UPDATE repo SET indexed_commit = head,
       indexed_seq = (SELECT max(seq) FROM commits c WHERE c.repo = repo.id);
     DROP INDEX commits_sha;
     CREATE UNIQUE INDEX commits_repo_sha ON commits (repo, sha);
     CREATE INDEX commits_sha ON commits (sha);",
)];

fn io_err(path: &Path) -> impl Fn(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn sql_err(path: &Path) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
    move |source| StoreError::Sqlite {
        path: path.display().to_string(),
        source,
    }
}

/// Where a rebuild or migration of `index` is written.
pub fn new_path(index: &Path) -> PathBuf {
    sidecar(index, ".new")
}

/// Deletes a `.new` left by a run that stopped part way, with its -wal and -shm. Only a regular
/// file owned by whoever owns the folder is removed; a symlink or someone else's file is an error.
pub fn remove_leftover(index: &Path) -> Result<(), StoreError> {
    let tmp = new_path(index);
    let m = match std::fs::symlink_metadata(&tmp) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_err(&tmp)(e)),
    };
    #[cfg(unix)]
    let foreign = {
        use std::os::unix::fs::MetadataExt;
        let dir = tmp.parent().unwrap_or(Path::new("."));
        let owner = std::fs::metadata(dir).map_err(io_err(dir))?.uid();
        m.uid() != owner
    };
    #[cfg(not(unix))]
    let foreign = false;
    if !m.file_type().is_file() || foreign {
        return Err(StoreError::Exists {
            path: tmp.display().to_string(),
        });
    }
    for p in [sidecar(&tmp, "-wal"), sidecar(&tmp, "-shm"), tmp.clone()] {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&p)(e)),
            _ => {}
        }
    }
    Ok(())
}

/// Renames a finished `.new` file over `index`. The `.new` file must be closed and checkpointed
/// (its -wal empty or gone).
///
/// SQLite finds a database's log by name, so the old file's -wal and -shm must be gone before the
/// new file takes the name: a reader that opened the new file while they were there would bind to
/// another file's logs. Under the writer lock (no other writer adds frames), the old file is first
/// checkpointed, best effort, so no committed frame lives only in its -wal; then its logs are
/// removed; then the new file is renamed in and the folder fsynced. A reader that lands in between
/// sees a complete old file or the new one. Connections already open keep their own handles.
pub fn install(index: &Path) -> Result<(), StoreError> {
    install_with(index, || {})
}

/// [`install`], running `between` after the old logs are gone and before the rename (tests).
pub(crate) fn install_with(index: &Path, between: impl FnOnce()) -> Result<(), StoreError> {
    let tmp = new_path(index);
    std::fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(io_err(&tmp))?;
    let logs = [sidecar(index, "-wal"), sidecar(index, "-shm")];
    let remove_logs = || -> Result<(), StoreError> {
        for p in &logs {
            match std::fs::remove_file(p) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(p)(e)),
                _ => {}
            }
        }
        Ok(())
    };
    // An old file that won't open is being replaced anyway.
    if index.exists()
        && let Ok(old) = Index::open(index, true)
    {
        old.conn
            .busy_timeout(std::time::Duration::from_secs(35))
            .map_err(sql_err(index))?;
        // Best effort: a reader holding an old snapshot can stop the log emptying, and then reads
        // through its own open handles; the frames it keeps only ever served the old file.
        let _ = old
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                r.get::<_, i64>(0)
            });
        continue_generation(&old.conn, &tmp)?;
    }
    remove_logs()?;
    between();
    std::fs::rename(&tmp, index).map_err(io_err(index))?;
    #[cfg(unix)]
    if let Some(dir) = index.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)
            .and_then(|f| f.sync_all())
            .map_err(io_err(dir))?;
    }
    Ok(())
}

/// Readers notice new data by `meta.generation` (SRV-02), and the app's cache and the server's
/// ticker key on it, but a new file counts from 0 again. The file that replaces an index carries on
/// from the old one's count, so a rebuild never lands on a number a reader has already seen.
fn continue_generation(old: &Connection, tmp: &Path) -> Result<(), StoreError> {
    let Ok(g) = old.query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
        r.get::<_, i64>(0)
    }) else {
        return Ok(());
    };
    {
        let new = Connection::open_with_flags(
            tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql_err(tmp))?;
        new.execute(
            "UPDATE meta SET value = max(value, ?1) WHERE key = 'generation'",
            [g + 1],
        )
        .map_err(sql_err(tmp))?;
        // Closing the only connection checkpoints its -wal into the file and removes it, as
        // install expects of the .new file.
    }
    std::fs::File::open(tmp)
        .and_then(|f| f.sync_all())
        .map_err(io_err(tmp))
}

/// Builds a new index with `build` and renames it over `index`, under the writer lock. `build`
/// gets the `.new` path, creates the file there (with [`Index::create`]) and closes it when done.
/// If it fails, the `.new` file is removed and the old index is untouched.
pub fn rebuild<T, E: From<StoreError>>(
    _lock: &WriterLock,
    index: &Path,
    build: impl FnOnce(&Path) -> Result<T, E>,
) -> Result<T, E> {
    remove_leftover(index)?;
    // ponytail: no free-space check before the build. A full disk fails the build with
    // SQLITE_FULL, the .new file is removed and the old index is untouched; checking up front
    // (current size plus 20%) needs statvfs, which std doesn't have.
    let out = match build(&new_path(index)) {
        Ok(out) => out,
        Err(e) => {
            let _ = remove_leftover(index);
            return Err(e);
        }
    };
    install(index)?;
    Ok(out)
}

/// Brings an older index to this binary's format: copy, migrate the copy, rename it in, never in
/// place. Returns false, changing nothing, when no registered path leads from its version: the
/// caller rebuilds.
pub fn migrate(lock: &WriterLock, index: &Path) -> Result<bool, StoreError> {
    migrate_with(lock, index, MIGRATIONS, FORMAT_VERSION)
}

/// [`migrate`] with its steps and target given, for tests.
pub fn migrate_with(
    _lock: &WriterLock,
    index: &Path,
    steps: &[Step],
    to: i64,
) -> Result<bool, StoreError> {
    let from = {
        let ix = Index::open_read(index)?;
        if ix.state != State::NeedsMigrate {
            return Ok(ix.state == State::Current);
        }
        ix.conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'format_version'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(sql_err(index))?
    };
    let path: Option<Vec<&str>> = (from..to)
        .map(|v| steps.iter().find(|s| s.0 == v).map(|s| s.1))
        .collect();
    let Some(path) = path else {
        return Ok(false);
    };
    remove_leftover(index)?;
    let tmp = new_path(index);
    let copy = || -> Result<(), StoreError> {
        {
            // VACUUM INTO writes a compacted copy; create the file first for O_EXCL and 0600.
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            opts.open(&tmp).map_err(io_err(&tmp))?;
            // The source connection stays SQLITE_OPEN_READONLY; query_only would refuse VACUUM
            // INTO, which writes only the copy.
            let src = Index::open_read(index)?;
            src.conn
                .execute_batch("PRAGMA query_only = OFF")
                .and_then(|()| src.conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy()]))
                .map_err(sql_err(index))?;
        }
        let conn = Connection::open(&tmp).map_err(sql_err(&tmp))?;
        harden(&conn, false).map_err(sql_err(&tmp))?;
        let run = |sql: &str| conn.execute_batch(sql).map_err(sql_err(&tmp));
        run("BEGIN IMMEDIATE")?;
        for sql in &path {
            run(sql)?;
        }
        run(&format!(
            "UPDATE meta SET value = {to} WHERE key = 'format_version';
             UPDATE meta SET value = value + 1 WHERE key = 'generation';
             PRAGMA user_version = {};
             COMMIT;",
            if to == FORMAT_VERSION {
                USER_VERSION
            } else {
                to
            }
        ))?;
        crate::writer::checkpoint(&conn).map_err(sql_err(&tmp))?;
        let check: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(sql_err(&tmp))?;
        if check != "ok" {
            return Err(StoreError::Damaged {
                path: tmp.display().to_string(),
                detail: check,
            });
        }
        Ok(())
    };
    if let Err(e) = copy() {
        let _ = remove_leftover(index);
        return Err(e);
    }
    install(index)?;
    tracing::info!(path = %index.display(), from, to, "index migrated");
    Ok(true)
}
