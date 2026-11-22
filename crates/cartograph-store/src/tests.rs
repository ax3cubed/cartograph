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
