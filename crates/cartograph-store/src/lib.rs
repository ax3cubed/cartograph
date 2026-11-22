//! The index file (IDX-03): schema v1, the open rules (B3) and the connection pragmas.
//!
//! An index file is untrusted input (board G01): the application id is checked, `quick_check` runs
//! before first use, the schema has no triggers or views, and every connection runs with
//! `trusted_schema = OFF`, defensive mode on and extension loading off.

use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use cartograph_core::Explain;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub use rusqlite;

/// `PRAGMA application_id`: "CART" (proposed value).
pub const APPLICATION_ID: i32 = 0x4341_5254;
/// The index format this binary writes and reads.
pub const FORMAT_VERSION: i64 = 1;
/// The oldest reader that can read what this binary writes.
pub const MIN_READER_VERSION: i64 = 1;
/// `PRAGMA user_version`.
pub const USER_VERSION: i64 = 1;
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
        harden(&conn, false).map_err(sql(path))?;
        conn.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {USER_VERSION};\n{SCHEMA}"
        ))
        .map_err(sql(path))?;
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

    fn open(path: &Path, write: bool) -> Result<Self, StoreError> {
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
        if n < 100 || &header[..16] != b"SQLite format 3\0" {
            return Err(StoreError::NotAnIndex { path: shown() });
        }
        if i32::from_be_bytes([header[68], header[69], header[70], header[71]]) != APPLICATION_ID {
            return Err(StoreError::NotAnIndex { path: shown() });
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let conn = Connection::open_with_flags(path, flags).map_err(sql(path))?;
        harden(&conn, !write).map_err(|e| damaged(e.to_string()))?;
        let check: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|e| damaged(e.to_string()))?;
        if check != "ok" {
            return Err(damaged(check));
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

/// The pragmas every connection gets (IDX-03).
fn harden(conn: &Connection, reader: bool) -> rusqlite::Result<()> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA trusted_schema = OFF; PRAGMA foreign_keys = ON;")?;
    if reader {
        conn.execute_batch("PRAGMA query_only = ON; PRAGMA mmap_size = 268435456;")?;
    } else {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
