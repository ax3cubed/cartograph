//! `cartograph index <dir>` writes one index under a temporary CARTOGRAPH_HOME and never
//! overwrites it.

use std::process::Command;

use cartograph_git::fixture::Fixture;
use cartograph_store::Index;

#[test]
fn index_writes_one_file_and_never_clobbers_it() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "import './b';");
    f.write("b.ts", "");
    f.commit("one");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_cartograph"))
            .arg("index")
            .arg(&f.dir)
            .env("CARTOGRAPH_HOME", &home)
            .output()
            .unwrap()
    };
    let first = run();
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert!(String::from_utf8_lossy(&first.stdout).contains("indexed 1 commits"));
    let ws: Vec<_> = std::fs::read_dir(home.join("workspaces"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(ws.len(), 1);
    let ix = Index::open_read(&ws[0].join("index.cartograph")).unwrap();
    let links: i64 = ix
        .conn
        .query_row(
            "SELECT count(*) FROM interval WHERE to_seq IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(links, 1);
    assert!(home.join("cache/facts.db").exists());

    let second = run();
    assert_eq!(second.status.code(), Some(1));
    let err = String::from_utf8_lossy(&second.stderr);
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(err.lines().count(), 3, "what, why, try");
}
