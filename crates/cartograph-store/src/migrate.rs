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
pub const MIGRATIONS: &[Step] = &[
    (
        1,
        "ALTER TABLE repo ADD COLUMN indexed_commit BLOB;
     ALTER TABLE repo ADD COLUMN indexed_seq INTEGER;
     ALTER TABLE repo ADD COLUMN removed INTEGER NOT NULL DEFAULT 0;
     UPDATE repo SET indexed_commit = head,
       indexed_seq = (SELECT max(seq) FROM commits c WHERE c.repo = repo.id);
     DROP INDEX commits_sha;
     CREATE UNIQUE INDEX commits_repo_sha ON commits (repo, sha);
     CREATE INDEX commits_sha ON commits (sha);",
    ),
    // v3 adds what phase 3 writes. History indexed before keeps one lineage per path (renames in
    // it read as a delete and an add); the derived rows (structure, counts, layout, search) are
    // empty, so the next sync builds them.
    (2, V3),
    // v4 adds phase 4's cross-service tables; each item's SQL is its own block in `V4`.
    (3, V4),
];

/// Takes a current file back to format 3 (test support): each item's undo is its own block, in the
/// reverse order of `V4`.
#[doc(hidden)]
pub const UNDO_V4: &str = concat!(
    // ---- XS-12 corrections: begin ----
    "DROP TABLE correction_interval; DROP TABLE correction_base; DROP TABLE correction;
     DROP INDEX link_dst; ALTER TABLE link DROP COLUMN layer;\n",
    // ---- XS-12 corrections: end ----
    // ---- XS-19 spec versions: begin ----
    "DROP TABLE contract_version;\n",
    // ---- XS-19 spec versions: end ----
    // ---- XS-11 generated clients: begin ----
    "DROP TABLE blob_ref; DROP TABLE client_alias; ALTER TABLE node DROP COLUMN generated;\n",
    // ---- XS-11 generated clients: end ----
    // ---- XS-08 contracts: begin ----
    "DROP TABLE binding; DROP TABLE contract_schema; DROP TABLE operation; DROP TABLE contract;\n",
    // ---- XS-08 contracts: end ----
    // ---- XS-09 config: begin ----
    "DROP TABLE link_config; DROP TABLE config_value; DROP TABLE environment;
     ALTER TABLE link DROP COLUMN envs;\n",
    // ---- XS-09 config: end ----
    // ---- XS-06 link detail: begin ----
    "ALTER TABLE evidence DROP COLUMN operation;\n",
    // ---- XS-06 link detail: end ----
    // ---- XS-05 confidence: begin ----
    "ALTER TABLE link DROP COLUMN score; ALTER TABLE link DROP COLUMN level;\n",
    // ---- XS-05 confidence: end ----
    // ---- XS-04 http links: begin ----
    "DROP INDEX fact_extractor; DROP INDEX file_version_blob;\n",
    // ---- XS-04 http links: end ----
    // ---- XS-02 host aliases: begin ----
    "DROP TABLE host_alias;
     ALTER TABLE node DROP COLUMN detect_source;
     CREATE TABLE alias (host TEXT NOT NULL, node INTEGER NOT NULL REFERENCES node (id),
       source TEXT NOT NULL) STRICT;\n",
    // ---- XS-02 host aliases: end ----
    // ---- XS-01 facts: begin ----
    "DROP TABLE evidence;
     ALTER TABLE unresolved DROP COLUMN fact_kind; ALTER TABLE unresolved DROP COLUMN raw;\n",
    // ---- XS-01 facts: end ----
    // ---- XS-23 owners: begin ----
    "DROP TABLE ownership; DROP TABLE codeowners;\n",
    // ---- XS-23 owners: end ----
);

/// Takes a current file back to format 2: format 4's additions, then format 3's.
#[doc(hidden)]
pub fn undo_to_v2() -> String {
    format!("{UNDO_V4} {UNDO_V3}")
}

/// The tables and columns format 4 adds, as `schema.sql` declares them; one delimited block per
/// phase-4 item.
const V4: &str = concat!(
    // ---- XS-12 corrections: begin (after the table it alters) ----
    "ALTER TABLE link ADD COLUMN layer TEXT;
     CREATE INDEX link_dst ON link (dst) WHERE kind = 'http';
     CREATE TABLE correction (id INTEGER PRIMARY KEY, line INTEGER NOT NULL, action TEXT NOT NULL,
       kind TEXT NOT NULL, key TEXT NOT NULL, src INTEGER, dst INTEGER, new_dst INTEGER,
       confidence TEXT, reason TEXT NOT NULL, anchor_seq INTEGER NOT NULL, status TEXT NOT NULL,
       links INTEGER NOT NULL, by TEXT, at INTEGER, fingerprint TEXT NOT NULL) STRICT;
     CREATE TABLE correction_base (id INTEGER PRIMARY KEY, link INTEGER NOT NULL UNIQUE REFERENCES link (id),
       correction INTEGER NOT NULL REFERENCES correction (id), added INTEGER NOT NULL,
       dst INTEGER NOT NULL, level TEXT, score REAL) STRICT;
     CREATE TABLE correction_interval (link INTEGER NOT NULL REFERENCES link (id),
       from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX correction_interval_link ON correction_interval (link);\n",
    // ---- XS-12 corrections: end ----
    // ---- XS-19 spec versions: begin ----
    "CREATE TABLE contract_version (contract INTEGER NOT NULL REFERENCES contract (id),
       n INTEGER NOT NULL, seq INTEGER NOT NULL, blob BLOB NOT NULL, spec_hash BLOB NOT NULL,
       changes INTEGER NOT NULL, breaking_count INTEGER NOT NULL, breaking TEXT NOT NULL,
       summary TEXT NOT NULL, PRIMARY KEY (contract, n)) STRICT, WITHOUT ROWID;\n",
    // ---- XS-19 spec versions: end ----
    // ---- XS-08 contracts: begin ----
    "CREATE TABLE contract (id INTEGER PRIMARY KEY, file INTEGER NOT NULL REFERENCES file (id),
       service INTEGER REFERENCES node (id), title TEXT NOT NULL, version TEXT NOT NULL) STRICT;
     CREATE UNIQUE INDEX contract_file ON contract (file);
     CREATE TABLE operation (id INTEGER PRIMARY KEY, contract INTEGER NOT NULL REFERENCES contract (id),
       method TEXT NOT NULL, path TEXT NOT NULL, template TEXT NOT NULL, operation_id TEXT,
       summary TEXT, line INTEGER NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX operation_contract ON operation (contract);
     CREATE TABLE contract_schema (contract INTEGER NOT NULL REFERENCES contract (id),
       name TEXT NOT NULL, fields TEXT NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX contract_schema_contract ON contract_schema (contract);
     CREATE TABLE binding (id INTEGER PRIMARY KEY, kind TEXT NOT NULL,
       operation INTEGER REFERENCES operation (id), file INTEGER NOT NULL REFERENCES file (id),
       line INTEGER NOT NULL, template TEXT NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER)
       STRICT;
     CREATE INDEX binding_operation ON binding (operation);
     CREATE INDEX binding_kind ON binding (kind, from_seq);\n",
    // ---- XS-08 contracts: end ----
    // ---- XS-05 confidence: begin ----
    "ALTER TABLE link ADD COLUMN level TEXT; ALTER TABLE link ADD COLUMN score REAL;\n",
    // ---- XS-05 confidence: end ----
    // ---- XS-04 http links: begin ----
    "CREATE INDEX fact_extractor ON fact (extractor);
     CREATE INDEX file_version_blob ON file_version (blob);\n",
    // ---- XS-04 http links: end ----
    // ---- XS-23 owners: begin ----
    "CREATE TABLE codeowners (repo INTEGER NOT NULL REFERENCES repo (id), blob BLOB NOT NULL,
       from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX codeowners_repo ON codeowners (repo, from_seq);
     CREATE TABLE ownership (file INTEGER NOT NULL REFERENCES file (id), owner TEXT NOT NULL,
       kind TEXT NOT NULL, pattern TEXT NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER)
       STRICT;
     CREATE INDEX ownership_file ON ownership (file, from_seq);
     CREATE INDEX ownership_owner ON ownership (owner);\n",
    // ---- XS-23 owners: end ----
    // ---- XS-01 facts: begin ----
    "ALTER TABLE unresolved ADD COLUMN fact_kind TEXT;
     ALTER TABLE unresolved ADD COLUMN raw TEXT;
     CREATE TABLE evidence (id INTEGER PRIMARY KEY, link INTEGER REFERENCES link (id),
       file INTEGER NOT NULL REFERENCES file (id), kind TEXT NOT NULL, text TEXT NOT NULL,
       start_line INTEGER NOT NULL, start_col INTEGER NOT NULL, end_line INTEGER NOT NULL,
       end_col INTEGER NOT NULL, extractor TEXT NOT NULL) STRICT;
     CREATE INDEX evidence_link ON evidence (link);
     CREATE INDEX evidence_file ON evidence (file);\n",
    // ---- XS-01 facts: end ----
    // ---- XS-06 link detail: begin (after the table it alters) ----
    "ALTER TABLE evidence ADD COLUMN operation TEXT;\n",
    // ---- XS-06 link detail: end ----
    // ---- XS-02 host aliases: begin ----
    "DROP TABLE alias;
     ALTER TABLE node ADD COLUMN detect_source TEXT;
     CREATE TABLE host_alias (host TEXT NOT NULL, port INTEGER, env_id INTEGER,
       service_id INTEGER NOT NULL REFERENCES node (id), source TEXT NOT NULL,
       blob_id BLOB NOT NULL, valid_from INTEGER NOT NULL, valid_to INTEGER) STRICT;
     CREATE INDEX host_alias_host ON host_alias (host, valid_from);
     CREATE INDEX host_alias_service ON host_alias (service_id);\n",
    // ---- XS-02 host aliases: end ----
    // ---- XS-09 config: begin ----
    "CREATE TABLE environment (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
       is_default INTEGER NOT NULL) STRICT;
     CREATE TABLE config_value (key TEXT NOT NULL, env_id INTEGER REFERENCES environment (id),
       value TEXT, value_kind TEXT NOT NULL, source TEXT NOT NULL, blob_id BLOB NOT NULL,
       valid_from INTEGER NOT NULL, valid_to INTEGER) STRICT;
     CREATE INDEX config_value_key ON config_value (key, valid_from);
     ALTER TABLE link ADD COLUMN envs INTEGER;
     CREATE TABLE link_config (link INTEGER NOT NULL REFERENCES link (id),
       env_id INTEGER NOT NULL REFERENCES environment (id), key TEXT NOT NULL, value TEXT NOT NULL,
       source TEXT NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX link_config_link ON link_config (link);\n",
    // ---- XS-09 config: end ----
    // ---- XS-11 generated clients: begin (after the tables it refers to) ----
    "CREATE TABLE client_alias (operation INTEGER NOT NULL REFERENCES operation (id),
       file INTEGER NOT NULL REFERENCES file (id), symbol TEXT NOT NULL, line INTEGER NOT NULL,
       from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX client_alias_operation ON client_alias (operation);
     CREATE TABLE blob_ref (blob BLOB PRIMARY KEY, version TEXT NOT NULL, payload BLOB NOT NULL)
       STRICT, WITHOUT ROWID;
     ALTER TABLE node ADD COLUMN generated TEXT;\n",
    // ---- XS-11 generated clients: end ----
);

/// Takes a format 3 file back to format 2 (test support: tests build an older file from a new one
/// by undoing each format's additions, newest first).
#[doc(hidden)]
pub const UNDO_V3: &str =
    "DROP TABLE search; DROP TABLE search_sub; DROP TABLE layout; DROP TABLE unresolved;
     DROP TABLE scope_count; DROP TABLE derived; DROP TABLE alias; DROP TABLE archived_interval;
     DROP TABLE rewrite; DROP TABLE note; DROP TABLE node_span;
     DROP INDEX node_key; DROP INDEX node_parent; DROP INDEX node_service;
     DELETE FROM interval WHERE link IN (SELECT l.id FROM link l JOIN node n
       ON n.id IN (l.src, l.dst) WHERE n.file IS NULL);
     DELETE FROM link WHERE src IN (SELECT id FROM node WHERE file IS NULL)
       OR dst IN (SELECT id FROM node WHERE file IS NULL);
     DELETE FROM node WHERE file IS NULL;
     ALTER TABLE node DROP COLUMN key; ALTER TABLE node DROP COLUMN name;
     ALTER TABLE node DROP COLUMN parent; ALTER TABLE node DROP COLUMN service;
     ALTER TABLE node DROP COLUMN detected_by; ALTER TABLE node DROP COLUMN fan_in;
     ALTER TABLE file_path DROP COLUMN via; ALTER TABLE file_path DROP COLUMN similarity;
     ALTER TABLE run DROP COLUMN files_total; ALTER TABLE run DROP COLUMN files_parsed;
     ALTER TABLE run DROP COLUMN imports_total; ALTER TABLE run DROP COLUMN imports_resolved;";

/// The tables and columns format 3 adds, as `schema.sql` declares them.
const V3: &str = "ALTER TABLE run ADD COLUMN files_total INTEGER;
     ALTER TABLE run ADD COLUMN files_parsed INTEGER;
     ALTER TABLE run ADD COLUMN imports_total INTEGER;
     ALTER TABLE run ADD COLUMN imports_resolved INTEGER;
     ALTER TABLE file_path ADD COLUMN via TEXT NOT NULL DEFAULT 'add';
     ALTER TABLE file_path ADD COLUMN similarity INTEGER;
     ALTER TABLE node ADD COLUMN key TEXT;
     ALTER TABLE node ADD COLUMN name TEXT;
     ALTER TABLE node ADD COLUMN parent INTEGER;
     ALTER TABLE node ADD COLUMN service INTEGER;
     ALTER TABLE node ADD COLUMN detected_by TEXT;
     ALTER TABLE node ADD COLUMN fan_in INTEGER NOT NULL DEFAULT 0;
     CREATE UNIQUE INDEX node_key ON node (key);
     CREATE INDEX node_parent ON node (parent);
     CREATE INDEX node_service ON node (service);
     CREATE TABLE node_span (node INTEGER NOT NULL REFERENCES node (id), from_seq INTEGER NOT NULL,
       to_seq INTEGER) STRICT;
     CREATE INDEX node_span_node ON node_span (node);
     CREATE TABLE note (seq INTEGER NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL) STRICT;
     CREATE INDEX note_seq ON note (seq);
     CREATE TABLE rewrite (id INTEGER PRIMARY KEY, repo INTEGER NOT NULL REFERENCES repo (id),
       old_head BLOB NOT NULL, new_head BLOB NOT NULL, base BLOB, at INTEGER,
       recorded_at INTEGER NOT NULL, archived INTEGER NOT NULL, dropped INTEGER NOT NULL) STRICT;
     CREATE TABLE archived_interval (rewrite INTEGER NOT NULL REFERENCES rewrite (id),
       link INTEGER NOT NULL REFERENCES link (id), from_sha BLOB NOT NULL, to_sha BLOB) STRICT;
     CREATE INDEX archived_interval_rewrite ON archived_interval (rewrite);
     CREATE TABLE alias (host TEXT NOT NULL, node INTEGER NOT NULL REFERENCES node (id),
       source TEXT NOT NULL) STRICT;
     CREATE TABLE derived (kind TEXT NOT NULL, scope TEXT NOT NULL, inputs_key BLOB NOT NULL,
       stale INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (kind, scope)) STRICT, WITHOUT ROWID;
     CREATE TABLE scope_count (node INTEGER PRIMARY KEY REFERENCES node (id),
       modules INTEGER NOT NULL, files INTEGER NOT NULL, links_out INTEGER NOT NULL,
       links_in INTEGER NOT NULL, links_inside INTEGER NOT NULL) STRICT;
     CREATE TABLE unresolved (file INTEGER NOT NULL REFERENCES file (id), spec TEXT NOT NULL,
       line INTEGER NOT NULL, stage TEXT NOT NULL, reason TEXT NOT NULL,
       from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX unresolved_at ON unresolved (from_seq, to_seq);
     CREATE INDEX unresolved_file ON unresolved (file, spec);
     CREATE TABLE layout (node INTEGER NOT NULL REFERENCES node (id), x REAL NOT NULL,
       y REAL NOT NULL, r REAL NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
     CREATE INDEX layout_node ON layout (node, from_seq);
     CREATE VIRTUAL TABLE search USING fts5 (name, split, path, docstring, summary,
       content = '', contentless_delete = 1, prefix = '2 3', tokenize = 'unicode61');
     CREATE VIRTUAL TABLE search_sub USING fts5 (name, path,
       content = '', contentless_delete = 1, tokenize = 'trigram');";

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
