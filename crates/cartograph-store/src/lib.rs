//! The index file (IDX-03): the schema, the open rules (B3) and the connection pragmas; the single
//! writer and its lock (IDX-13, [`writer`]); migrations and the atomic rename (IDX-16,
//! [`migrate`]); the fact hash (IDX-17, [`hash`]); derived rows (IDX-21, [`derived`]); search
//! (AI-15, [`search`]).
//!
//! An index file is untrusted input (board G01): the application id is checked, `quick_check` runs
//! before first use, the schema has no triggers or views, and every connection runs with
//! `trusted_schema = OFF`, defensive mode on and extension loading off.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use cartograph_core::Explain;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub mod derived;
pub mod hash;
pub mod migrate;
pub mod search;
pub mod writer;

pub use hash::{FactHash, fact_hash};
pub use rusqlite;

/// `PRAGMA application_id`: "CART" (proposed value).
pub const APPLICATION_ID: i32 = 0x4341_5254;
/// The index format this binary writes and reads.
pub const FORMAT_VERSION: i64 = 3;
/// The oldest reader that can read what this binary writes: v2 and v3 only add tables and columns,
/// and an older reader's queries mean the same on them.
pub const MIN_READER_VERSION: i64 = 1;
/// `PRAGMA user_version`, kept equal to the format version.
pub const USER_VERSION: i64 = FORMAT_VERSION;
pub const SCHEMA: &str = include_str!("schema.sql");

/// What the open rules decided about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Same format as this binary.
    Current,
    /// Newer, but readable by this binary: open read-only, with a warning.
    NewerReadable,
    /// Older: read-only until IDX-16 migrates or rebuilds it.
    NeedsMigrate,
}

/// An open index.
pub struct Index {
    pub conn: Connection,
    pub state: State,
    pub path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path} already exists")]
    Exists { path: String },
    #[error("{path} is not a Cartograph index")]
    NotAnIndex { path: String },
    #[error("{path} is damaged")]
    Damaged { path: String, detail: String },
    #[error("{path} needs a newer Cartograph (index format {format}, readable from {min_reader})")]
    TooNew {
        path: String,
        format: i64,
        min_reader: i64,
    },
    #[error("{path} can't be written in its state ({state:?})")]
    ReadOnly { path: String, state: State },
    #[error("{path} is held open by a reader")]
    Busy { path: String },
    #[error("SQLite error on {path}")]
    Sqlite {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error("can't create {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl Explain for StoreError {
    fn why(&self) -> String {
        match self {
            Self::Exists { .. } => "an existing index is never overwritten".into(),
            Self::NotAnIndex { .. } => "its SQLite application id isn't Cartograph's".into(),
            Self::Damaged { detail, .. } => {
                format!("the file is truncated or fails quick_check: {detail}")
            }
            Self::TooNew { .. } => {
                "a newer Cartograph wrote it in a format this one can't read".into()
            }
            Self::ReadOnly { state, .. } => match state {
                State::NeedsMigrate => "it was written by an older Cartograph".into(),
                _ => "it was written by a newer Cartograph".into(),
            },
            Self::Busy { .. } => {
                "a reader kept an old snapshot past the 30 s limit, so its log can't be emptied"
                    .into()
            }
            Self::Sqlite { source, .. } => source.to_string(),
            Self::Io { source, .. } => source.to_string(),
        }
    }
    fn action(&self) -> String {
        match self {
            Self::Exists { path } => format!("delete {path}, or choose another output path"),
            Self::NotAnIndex { .. } => "open an index.cartograph file".into(),
            Self::Damaged { .. } | Self::ReadOnly { .. } => "cartograph index --rebuild".into(),
            Self::TooNew { .. } => "upgrade Cartograph to the latest version".into(),
            Self::Busy { .. } => {
                "close the app and MCP clients of this workspace, then retry".into()
            }
            Self::Sqlite { .. } | Self::Io { .. } => {
                "check the disk and the folder's permissions".into()
            }
        }
    }
}

fn sql(path: &Path) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
    move |source| StoreError::Sqlite {
        path: path.display().to_string(),
        source,
    }
}

impl Index {
    /// Creates a new index. The file is created with O_EXCL (and 0600 on Unix), so an existing file
    /// is never clobbered.
    pub fn create(path: &Path) -> Result<Self, StoreError> {
        let shown = || path.display().to_string();
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(path).map_err(|source| match source.kind() {
            std::io::ErrorKind::AlreadyExists => StoreError::Exists { path: shown() },
            _ => StoreError::Io {
                path: shown(),
                source,
            },
        })?;
        let conn = Connection::open(path).map_err(sql(path))?;
        // The identity goes into the main file before WAL is on: `open` reads it from the header on
        // disk, and in WAL mode page 1 would sit in the -wal until the first checkpoint.
        conn.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {USER_VERSION};"
        ))
        .map_err(sql(path))?;
        harden(&conn, false).map_err(sql(path))?;
        conn.execute_batch(SCHEMA).map_err(sql(path))?;
        for (k, v) in [
            ("format_version", FORMAT_VERSION),
            ("min_reader_version", MIN_READER_VERSION),
            ("generation", 0),
        ] {
            conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2)", (k, v))
                .map_err(sql(path))?;
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('writer_version', ?1), ('state', 'building')",
            [env!("CARGO_PKG_VERSION")],
        )
        .map_err(sql(path))?;
        Ok(Self {
            conn,
            state: State::Current,
            path: path.to_owned(),
        })
    }

    /// Opens for reading. A newer-but-readable or older file opens read-only with its state set.
    pub fn open_read(path: &Path) -> Result<Self, StoreError> {
        Self::open(path, false)
    }

    /// Opens for writing. Only a file in this binary's format can be written.
    pub fn open_write(path: &Path) -> Result<Self, StoreError> {
        let ix = Self::open(path, true)?;
        if ix.state != State::Current {
            return Err(StoreError::ReadOnly {
                path: path.display().to_string(),
                state: ix.state,
            });
        }
        Ok(ix)
    }

    pub(crate) fn open(path: &Path, write: bool) -> Result<Self, StoreError> {
        let shown = || path.display().to_string();
        let damaged = |detail: String| StoreError::Damaged {
            path: shown(),
            detail,
        };
        // SQLite treats an empty or missing file as a new database; an index is never that.
        let mut header = [0u8; 100];
        let n = std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut header))
            .map_err(|source| StoreError::Io {
                path: shown(),
                source,
            })?;
        // Whose file this is comes from the header on disk (the magic, then the application id at
        // offset 68, big-endian), before any connection opens: a write-mode open runs pragmas, and
        // closing it would checkpoint another program's -wal into its file. `create` writes the id
        // into the main file before WAL is on, so an index's header always carries it.
        let magic = n >= 16 && &header[..16] == b"SQLite format 3\0";
        let app_here = n >= 72
            && i32::from_be_bytes([header[68], header[69], header[70], header[71]])
                == APPLICATION_ID;
        if !(magic && app_here) {
            return Err(StoreError::NotAnIndex { path: shown() });
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let conn = Connection::open_with_flags(path, flags).map_err(sql(path))?;
        harden(&conn, !write).map_err(|e| damaged(e.to_string()))?;
        let app: i32 = conn
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(|e| damaged(e.to_string()))?;
        if app != APPLICATION_ID {
            return Err(StoreError::NotAnIndex { path: shown() });
        }
        // quick_check reads every page (about 440 ms on remotion), so it runs once per file per
        // process, not once per pooled connection. A file renamed into place is a new file.
        let key = file_key(path);
        let seen = key.is_some_and(|k| CHECKED.lock().is_ok_and(|c| c.contains(&k)));
        if !seen {
            let check: String = conn
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .map_err(|e| damaged(e.to_string()))?;
            if check != "ok" {
                return Err(damaged(check));
            }
            if let (Some(k), Ok(mut c)) = (key, CHECKED.lock()) {
                c.insert(k);
            }
        }
        let meta = |k: &str| -> Result<Option<i64>, StoreError> {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
                .optional()
                .map_err(|e| damaged(e.to_string()))
        };
        let format = meta("format_version")?.ok_or_else(|| damaged("no format_version".into()))?;
        let min_reader = meta("min_reader_version")?.unwrap_or(format);
        let state = if format == FORMAT_VERSION {
            State::Current
        } else if format > FORMAT_VERSION {
            if min_reader > FORMAT_VERSION {
                return Err(StoreError::TooNew {
                    path: shown(),
                    format,
                    min_reader,
                });
            }
            tracing::warn!(path = %path.display(), format, "index is newer than this binary; opened read-only");
            State::NewerReadable
        } else {
            State::NeedsMigrate
        };
        if state != State::Current {
            conn.pragma_update(None, "query_only", true)
                .map_err(sql(path))?;
        }
        Ok(Self {
            conn,
            state,
            path: path.to_owned(),
        })
    }
}

/// The workspace's (services, modules, files) live at the last step, in one query: what
/// WorkspaceSummary shows (IDX-20). Services count services and libraries, the map's top level.
pub fn summary_counts(conn: &Connection) -> rusqlite::Result<(i64, i64, i64)> {
    conn.query_row(
        "SELECT
           (SELECT count(*) FROM node n WHERE n.kind IN ('service', 'library')
              AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id AND s.to_seq IS NULL)),
           (SELECT count(*) FROM node n WHERE n.kind = 'module'
              AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id AND s.to_seq IS NULL)),
           (SELECT count(*) FROM file_path WHERE to_seq IS NULL)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

/// Files whose `quick_check` passed in this process.
static CHECKED: LazyLock<Mutex<HashSet<FileKey>>> = LazyLock::new(Default::default);

/// Device, inode and birth time: a different value for a file renamed into place, and for a new
/// file that reuses a freed inode.
type FileKey = (u64, u64, u128);

fn file_key(path: &Path) -> Option<FileKey> {
    let m = std::fs::metadata(path).ok()?;
    let born = m
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((m.dev(), m.ino(), born))
    }
    #[cfg(not(unix))]
    {
        // ponytail: no inode in std on Windows, so every open runs quick_check there. Key on the
        // file index from GetFileInformationByHandle when Windows readers show up in profiles.
        let _ = born;
        None
    }
}

/// `index.cartograph` plus a suffix: `-wal`, `-shm`, `.new`.
pub fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The pragmas every connection gets (IDX-03).
fn harden(conn: &Connection, reader: bool) -> rusqlite::Result<()> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA trusted_schema = OFF; PRAGMA foreign_keys = ON;")?;
    if reader {
        // A reader never checkpoints, not even on close: after a rebuild renames a new file into
        // place, the -wal next to it is the new file's (IDX-16).
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)?;
        conn.execute_batch("PRAGMA query_only = ON; PRAGMA mmap_size = 268435456;")?;
    } else {
        // journal_size_limit bounds the WAL a long reader can make the writer keep (IDX-13).
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
             PRAGMA journal_size_limit = 67108864;",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
