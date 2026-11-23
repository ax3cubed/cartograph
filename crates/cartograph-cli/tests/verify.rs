//! `cartograph verify` (IDX-17): equal facts exit 0; a difference exits 1 and names the section.

use std::path::Path;
use std::process::{Command, Output};

use cartograph_git::fixture::Fixture;
use cartograph_store::rusqlite::Connection;

fn cartograph(home: &Path, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .output()
        .unwrap()
}

#[test]
fn verify_compares_fact_hashes() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "import './b';");
    f.write("b.ts", "");
    f.commit("one");
    f.write("c.ts", "import './a';");
    f.commit("two");
    let other = d.path().join("other.cartograph");
    let built = cartograph(home.as_path(), &["index".as_ref(), f.dir.as_os_str()]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let built = cartograph(
        home.as_path(),
        &[
            "index".as_ref(),
            f.dir.as_os_str(),
            "--out".as_ref(),
            other.as_os_str(),
        ],
    );
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let dir = ["--dir".as_ref(), f.dir.as_os_str()];

    // No argument: the index against the hash stored at its last build.
    let own = cartograph(&home, &[&["verify".as_ref()][..], &dir].concat());
    assert_eq!(own.status.code(), Some(0), "{own:?}");
    assert!(String::from_utf8_lossy(&own.stdout).contains("same facts: sha256:"));

    let same = cartograph(
        &home,
        &[&["verify".as_ref(), other.as_os_str()][..], &dir].concat(),
    );
    assert_eq!(same.status.code(), Some(0), "{same:?}");

    Connection::open(&other)
        .unwrap()
        .execute_batch("UPDATE interval SET from_seq = from_seq + 1 WHERE from_seq = 0")
        .unwrap();
    let differ = cartograph(
        &home,
        &[&["verify".as_ref(), other.as_os_str()][..], &dir].concat(),
    );
    assert_eq!(differ.status.code(), Some(1), "{differ:?}");
    let err = String::from_utf8_lossy(&differ.stderr);
    assert!(
        err.contains("different facts") && err.contains("links"),
        "{err}"
    );
}

#[test]
fn verify_during_a_sync_says_so_instead_of_reporting_different_facts() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "import './b';");
    f.write("b.ts", "");
    f.commit("one");
    let built = cartograph(&home, &["index".as_ref(), f.dir.as_os_str()]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let index = std::fs::read_dir(home.join("workspaces"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("index.cartograph"))
        .find(|p| p.exists())
        .unwrap();
    // A sync commits batches while the state says syncing; the stored hash is the last complete one.
    Connection::open(&index)
        .unwrap()
        .execute_batch("UPDATE meta SET value = 'syncing' WHERE key = 'state'")
        .unwrap();
    let o = cartograph(
        &home,
        &["verify".as_ref(), "--dir".as_ref(), f.dir.as_os_str()],
    );
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("part way through a sync") && !err.contains("different facts"),
        "{err}"
    );
}
