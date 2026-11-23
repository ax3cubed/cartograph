use super::*;
use proptest::prelude::*;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A valid index whose meta says `format` and `min_reader`.
fn with_format(dir: &Path, format: i64, min_reader: i64) -> PathBuf {
    let p = dir.join("index.cartograph");
    let ix = Index::create(&p).unwrap();
    ix.conn
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = 'format_version'",
            [format],
        )
        .unwrap();
    ix.conn
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = 'min_reader_version'",
            [min_reader],
        )
        .unwrap();
    drop(ix);
    p
}

#[test]
fn created_file_reports_application_id_and_user_version() {
    let d = tmp();
    let ix = Index::create(&d.path().join("i.cartograph")).unwrap();
    let app: i32 = ix
        .conn
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    let user: i64 = ix
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!((app, user), (APPLICATION_ID, USER_VERSION));
    let mode: String = ix
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn schema_matches_the_golden_file() {
    let d = tmp();
    let ix = Index::create(&d.path().join("i.cartograph")).unwrap();
    let mut st = ix
        .conn
        .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type DESC, name")
        .unwrap();
    let rows: Vec<String> = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    insta::assert_snapshot!(rows.join(";\n"));
}

#[test]
#[cfg(unix)]
fn create_never_clobbers_and_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp();
    let p = d.path().join("i.cartograph");
    drop(Index::create(&p).unwrap());
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(matches!(Index::create(&p), Err(StoreError::Exists { .. })));
}

// The open-rules table (B3), one fixture per row.

#[test]
fn rule_wrong_application_id() {
    let d = tmp();
    let p = d.path().join("other.db");
    Connection::open(&p)
        .unwrap()
        .execute_batch("CREATE TABLE t (x); PRAGMA application_id = 7;")
        .unwrap();
    let e = Index::open_read(&p).err().unwrap();
    assert!(matches!(e, StoreError::NotAnIndex { .. }));
    assert!(e.to_string().contains("not a Cartograph index"));
}

#[test]
fn rule_same_format_opens() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION, MIN_READER_VERSION);
    assert_eq!(Index::open_write(&p).unwrap().state, State::Current);
    assert_eq!(Index::open_read(&p).unwrap().state, State::Current);
}

#[test]
fn rule_newer_but_readable_opens_read_only() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION + 1, FORMAT_VERSION);
    let ix = Index::open_read(&p).unwrap();
    assert_eq!(ix.state, State::NewerReadable);
    assert!(ix.conn.execute("DELETE FROM meta", []).is_err());
    assert!(matches!(
        Index::open_write(&p),
        Err(StoreError::ReadOnly { .. })
    ));
}

#[test]
fn rule_newer_and_unreadable_names_the_upgrade() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION + 1, FORMAT_VERSION + 1);
    let e = Index::open_read(&p).err().unwrap();
    assert!(matches!(e, StoreError::TooNew { .. }));
    assert!(e.action().contains("upgrade"));
    assert_eq!(e.exit_code(), cartograph_core::ExitCode::Error);
}

#[test]
fn rule_older_needs_migrate() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION - 1, FORMAT_VERSION - 1);
    let ix = Index::open_read(&p).unwrap();
    assert_eq!(ix.state, State::NeedsMigrate);
    assert!(matches!(
        Index::open_write(&p),
        Err(StoreError::ReadOnly {
            state: State::NeedsMigrate,
            ..
        })
    ));
}

#[test]
fn rule_truncated_file_is_damaged() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION, MIN_READER_VERSION);
    // Fold the WAL into the main file, then cut the file in half.
    Connection::open(&p)
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
        .unwrap();
    let bytes = std::fs::read(&p).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() / 2]).unwrap();
    let e = Index::open_read(&p).err().unwrap();
    assert!(matches!(e, StoreError::Damaged { .. }), "{e:?}");
    assert_eq!(e.action(), "cartograph index --rebuild");
}

proptest! {
    // The fuzz target, as a property: arbitrary bytes give an error, never a panic.
    // ponytail: proptest instead of cargo-fuzz, which needs nightly; a cargo-fuzz target comes with HD-06.
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096), keep_header in any::<bool>()) {
        let d = tmp();
        let p = d.path().join("x.cartograph");
        let mut b = bytes;
        if keep_header && b.len() >= 100 {
            b[..16].copy_from_slice(b"SQLite format 3\0");
            b[68..72].copy_from_slice(&APPLICATION_ID.to_be_bytes());
        }
        std::fs::write(&p, &b).unwrap();
        prop_assert!(Index::open_read(&p).is_err());
    }
}

// Phase 1 review fixes.

#[test]
fn a_wal_file_never_checkpointed_opens() {
    let d = tmp();
    let p = d.path().join("i.cartograph");
    // The writer stays open, so nothing has been checkpointed into the main file yet.
    let w = Index::create(&p).unwrap();
    w.conn
        .execute(
            "INSERT INTO repo (name, path, shallow) VALUES ('r', '.', 0)",
            [],
        )
        .unwrap();
    assert!(std::fs::metadata(sidecar(&p, "-wal")).unwrap().len() > 0);
    // The id is in the main file's header from creation, written before WAL is on, so the header
    // check `open` makes before touching a file passes while page 1 is still in the -wal.
    let main = std::fs::read(&p).unwrap();
    assert_eq!(main[68..72], APPLICATION_ID.to_be_bytes());
    let r = Index::open_read(&p).unwrap();
    let n: i64 = r
        .conn
        .query_row("SELECT count(*) FROM repo", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    // An empty file with no WAL is still not an index.
    let e = d.path().join("empty.cartograph");
    std::fs::write(&e, b"").unwrap();
    assert!(matches!(
        Index::open_read(&e),
        Err(StoreError::NotAnIndex { .. })
    ));
}

#[test]
fn quick_check_runs_once_per_file() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION, MIN_READER_VERSION);
    Connection::open(&p)
        .unwrap()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS pad (x); PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode = DELETE;",
        )
        .unwrap();
    drop(Index::open_read(&p).unwrap());
    // Damage the last page in place, the same inode: this process already checked the file, so
    // the next connection doesn't read every page again.
    let mut bytes = std::fs::read(&p).unwrap();
    let n = bytes.len();
    bytes[n - 4096..].fill(0xa5);
    std::fs::write(&p, &bytes).unwrap();
    assert!(Index::open_read(&p).is_ok());
    // A copy is a new file, and gets checked.
    let q = d.path().join("copy.cartograph");
    std::fs::write(&q, &bytes).unwrap();
    assert!(matches!(
        Index::open_read(&q),
        Err(StoreError::Damaged { .. })
    ));
}

// IDX-13: the writer lock, the dirty flag and batches.

#[test]
fn one_writer_at_a_time_and_a_dead_holder_does_not_block() {
    use writer::*;
    let d = tmp();
    let a = WriterLock::try_acquire(d.path()).unwrap().expect("free");
    assert!(WriterLock::try_acquire(d.path()).unwrap().is_none());
    drop(a);
    // A lock file left with a dead pid is only text: the OS lock is what counts.
    std::fs::write(d.path().join(LOCK_FILE), "pid 999999\nsince 0\n").unwrap();
    let b = WriterLock::try_acquire(d.path()).unwrap();
    assert!(b.is_some());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.path().join(LOCK_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}

#[test]
fn a_request_during_a_run_is_one_more_round_not_a_second_writer() {
    use writer::*;
    let d = tmp();
    let mut nested = Vec::new();
    let out = run_exclusive::<_, std::io::Error>(d.path(), None, |_| {
        if nested.is_empty() {
            // Two more requests while this round runs: both return at once.
            nested.push(run_exclusive::<(), std::io::Error>(d.path(), None, |_| Ok(())).unwrap());
            nested.push(run_exclusive::<(), std::io::Error>(d.path(), None, |_| Ok(())).unwrap());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(nested, [Exclusive::Busy, Exclusive::Busy]);
    assert_eq!(
        out,
        Exclusive::Ran {
            last: (),
            rounds: 2
        }
    );
    assert!(!d.path().join(DIRTY_FILE).exists());
}

#[test]
fn cancel_stops_after_the_current_round_and_a_stale_cancel_is_ignored() {
    use writer::*;
    let d = tmp();
    request_cancel(d.path()).unwrap();
    let mut seen = Vec::new();
    let out = run_exclusive::<_, std::io::Error>(d.path(), None, |w| {
        seen.push(w.cancel_requested());
        mark_dirty(d.path())?;
        request_cancel(d.path())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, [false], "the cancel from before the run was dropped");
    assert_eq!(
        out,
        Exclusive::Ran {
            last: (),
            rounds: 1
        }
    );
}

#[test]
fn progress_is_written_atomically_and_throttled() {
    use writer::*;
    let d = tmp();
    let w = WriterLock::try_acquire(d.path()).unwrap().unwrap();
    let p = |done| Progress {
        phase: "replay",
        done,
        total: 10,
        repo: Some("a\"b"),
    };
    w.progress(&p(1), false).unwrap();
    w.progress(&p(2), false).unwrap();
    let read = || std::fs::read_to_string(d.path().join(PROGRESS_FILE)).unwrap();
    assert!(read().contains(r#""done":1"#), "{}", read());
    assert!(read().contains(r#""repo":"a\"b""#));
    w.progress(&p(3), true).unwrap();
    assert!(read().contains(r#""done":3"#));
}

#[test]
fn every_batch_bumps_the_generation_and_a_reader_keeps_its_snapshot() {
    use writer::*;
    let d = tmp();
    let p = d.path().join("i.cartograph");
    let w = Index::create(&p).unwrap();
    let r = Index::open_read(&p).unwrap();
    let generation = |c: &Connection| -> i64 {
        c.query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let repos = |c: &Connection| -> i64 {
        c.query_row("SELECT count(*) FROM repo", [], |r| r.get(0))
            .unwrap()
    };
    r.conn.execute_batch("BEGIN").unwrap();
    assert_eq!((generation(&r.conn), repos(&r.conn)), (0, 0));
    let mut b = Batches::begin(&w.conn).unwrap();
    for i in 0..3 {
        w.conn
            .execute(
                "INSERT INTO repo (name, path, shallow) VALUES (?1, '.', 0)",
                [i],
            )
            .unwrap();
        b.commit(&w.conn).unwrap();
        // The reader's transaction started before the first commit: same rows throughout.
        assert_eq!((generation(&r.conn), repos(&r.conn)), (0, 0));
    }
    b.finish(&w.conn).unwrap();
    r.conn.execute_batch("COMMIT").unwrap();
    assert_eq!((generation(&r.conn), repos(&r.conn)), (4, 3));
}

#[test]
fn a_run_that_died_is_marked_failed() {
    let d = tmp();
    let w = Index::create(&d.path().join("i.cartograph")).unwrap();
    w.conn
        .execute("UPDATE meta SET value = 'syncing' WHERE key = 'state'", [])
        .unwrap();
    assert_eq!(
        writer::recover(&w.conn).unwrap().as_deref(),
        Some("syncing")
    );
    let (state, why): (String, String) = w
        .conn
        .query_row(
            "SELECT (SELECT value FROM meta WHERE key = 'state'),
                    (SELECT value FROM meta WHERE key = 'last_error')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert!(why.contains("part way"));
}

// IDX-16: a migration on a copy, and a version with no migration.

#[test]
fn migrate_runs_on_a_copy_and_an_unknown_version_means_rebuild() {
    let d = tmp();
    let p = with_format(d.path(), FORMAT_VERSION, MIN_READER_VERSION);
    let lock = writer::WriterLock::try_acquire(d.path()).unwrap().unwrap();
    // Pretend this file is one format older, in a world where the last step adds a table.
    Connection::open(&p)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE meta SET value = {} WHERE key = 'format_version'",
            FORMAT_VERSION - 1
        ))
        .unwrap();
    let ino = |p: &Path| file_key(p).unwrap();
    let before = ino(&p);
    let reader = Index::open_read(&p).unwrap();
    assert_eq!(reader.state, State::NeedsMigrate);
    let steps: &[migrate::Step] = &[(FORMAT_VERSION - 1, "CREATE TABLE added (x INTEGER) STRICT;")];
    assert!(migrate::migrate_with(&lock, &p, steps, FORMAT_VERSION).unwrap());
    assert_ne!(ino(&p), before, "renamed in, not changed in place");
    // The reader that had the old file still reads the old file.
    let old: i64 = reader
        .conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'added'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 0);
    let ix = Index::open_read(&p).unwrap();
    assert_eq!(ix.state, State::Current);
    let (v, user): (i64, i64) = ix
        .conn
        .query_row(
            "SELECT (SELECT value FROM meta WHERE key = 'format_version'),
                    (SELECT user_version FROM pragma_user_version)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((v, user), (FORMAT_VERSION, USER_VERSION));
    assert!(!migrate::new_path(&p).exists());
    // Format 0 has no registered step: nothing changes and the caller rebuilds.
    Connection::open(&p)
        .unwrap()
        .execute_batch("UPDATE meta SET value = 0 WHERE key = 'format_version'")
        .unwrap();
    assert!(!migrate::migrate(&lock, &p).unwrap());
}

#[test]
#[cfg(unix)]
fn a_leftover_new_file_is_removed_only_if_it_is_a_plain_file() {
    let d = tmp();
    let p = d.path().join("index.cartograph");
    let new = migrate::new_path(&p);
    std::fs::write(&new, b"half").unwrap();
    std::fs::write(sidecar(&new, "-wal"), b"half").unwrap();
    migrate::remove_leftover(&p).unwrap();
    assert!(!new.exists() && !sidecar(&new, "-wal").exists());
    let target = d.path().join("precious");
    std::fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, &new).unwrap();
    assert!(migrate::remove_leftover(&p).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
}

#[test]
fn max_rounds_caps_the_rounds_and_keeps_the_flag() {
    use writer::*;
    let d = tmp();
    let out = run_exclusive::<_, std::io::Error>(d.path(), Some(3), |_| {
        // A request lands during every round.
        mark_dirty(d.path())
    })
    .unwrap();
    assert_eq!(
        out,
        Exclusive::Ran {
            last: (),
            rounds: 3
        }
    );
    assert!(
        d.path().join(DIRTY_FILE).exists(),
        "left for the next request"
    );
}

#[test]
fn one_commit_in_two_repos() {
    let d = tmp();
    let ix = Index::create(&d.path().join("i.cartograph")).unwrap();
    let c = &ix.conn;
    c.execute_batch(
        "INSERT INTO repo (id, name, path, shallow) VALUES (1, 'a', 'a', 0), (2, 'fork', 'b', 0);
         INSERT INTO commits VALUES (0, 1, x'01', 0, 0, 'x', 'one'), (1, 2, x'01', 0, 1, 'x', 'one');",
    )
    .unwrap();
    // Twice in one repo is still refused.
    assert!(
        c.execute(
            "INSERT INTO commits VALUES (2, 1, x'01', 0, 2, 'x', 'one')",
            []
        )
        .is_err()
    );
    let h = fact_hash(c).unwrap();
    assert!(h.sections.iter().any(|s| s.label == "repo 1 commit"));
}

#[test]
fn a_rebuilt_index_carries_on_from_the_old_generation() {
    let d = tmp();
    let p = d.path().join("index.cartograph");
    let generation = |p: &Path| -> i64 {
        Index::open_read(p)
            .unwrap()
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    // An index that has seen five batches, and a rebuild that wrote one: a new file counts from 0.
    let old = Index::create(&p).unwrap();
    old.conn
        .execute("UPDATE meta SET value = 5 WHERE key = 'generation'", [])
        .unwrap();
    drop(old);
    let new = Index::create(&migrate::new_path(&p)).unwrap();
    new.conn
        .execute("UPDATE meta SET value = 1 WHERE key = 'generation'", [])
        .unwrap();
    drop(new);
    migrate::install(&p).unwrap();
    // Readers and the app's cache key on the generation: the same or a lower number would read as
    // nothing changed.
    assert_eq!(generation(&p), 6);
}

#[test]
fn a_foreign_wal_database_is_refused_and_left_untouched() {
    let d = tmp();
    // Another program's WAL-mode database, its committed frames still only in the -wal and its
    // owner gone: a copy of its files taken while it was open.
    let live = d.path().join("live.db");
    let owner = Connection::open(&live).unwrap();
    owner
        .execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
             PRAGMA application_id = 1234; CREATE TABLE customers (n);
             INSERT INTO customers VALUES (1), (2);",
        )
        .unwrap();
    let p = d.path().join("index.cartograph");
    std::fs::copy(&live, &p).unwrap();
    std::fs::copy(sidecar(&live, "-wal"), sidecar(&p, "-wal")).unwrap();
    drop(owner);
    let main = std::fs::read(&p).unwrap();
    let wal = std::fs::read(sidecar(&p, "-wal")).unwrap();
    assert!(!wal.is_empty(), "its frames are only in the -wal");
    for write in [true, false] {
        let e = Index::open(&p, write).err().unwrap();
        assert!(matches!(e, StoreError::NotAnIndex { .. }), "{e}");
    }
    // Refused from the header on disk: nothing checkpointed into the file, nothing deleted.
    assert_eq!(std::fs::read(&p).unwrap(), main);
    assert_eq!(std::fs::read(sidecar(&p, "-wal")).unwrap(), wal);
}

// Unix: the old file's writer stays open across the install, and Windows can't delete open files.
#[cfg(unix)]
#[test]
fn install_removes_the_old_logs_before_the_new_file_takes_the_name() {
    let d = tmp();
    let p = d.path().join("index.cartograph");
    let version = |c: &Connection| -> String {
        c.query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = 'writer_version'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    // The old index, its last commit still only in its -wal: its writer is open, nothing checkpointed.
    let old = Index::create(&p).unwrap();
    old.conn
        .execute(
            "UPDATE meta SET value = 'old' WHERE key = 'writer_version'",
            [],
        )
        .unwrap();
    let new = Index::create(&migrate::new_path(&p)).unwrap();
    new.conn
        .execute(
            "UPDATE meta SET value = 'new' WHERE key = 'writer_version'",
            [],
        )
        .unwrap();
    drop(new);
    let mut gap = None;
    migrate::install_with(&p, || {
        // Before the rename the old logs are gone, so the new file can't inherit them ...
        assert!(!sidecar(&p, "-wal").exists() && !sidecar(&p, "-shm").exists());
        // ... and a reader landing here reads the whole old file: its frames were checkpointed in.
        gap = Some(version(&Index::open_read(&p).unwrap().conn));
    })
    .unwrap();
    assert_eq!(gap.as_deref(), Some("old"));
    drop(old);
    assert_eq!(version(&Index::open_read(&p).unwrap().conn), "new");
}
