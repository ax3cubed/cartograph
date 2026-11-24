//! XS-10's exit gate: no planted secret survives in any text column of the index.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1rwW1gFWFOEjXkE";
const DB_USERINFO: &str = "user:pw@db";
const GH_TOKEN: &str = "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";

/// Every text column of every ordinary table, as (table, column).
fn text_columns(ix: &Index) -> Vec<(String, String)> {
    let tables: Vec<String> = ix
        .conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND sql NOT LIKE 'CREATE VIRTUAL%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = Vec::new();
    for t in tables {
        let cols: Vec<String> = ix
            .conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{t}') WHERE type = 'TEXT'"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        out.extend(cols.into_iter().map(|c| (t.clone(), c)));
    }
    out
}

#[test]
fn no_planted_secret_survives_in_any_text_column_of_the_index() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    // A database URL with a password and an AWS key in specifiers that don't resolve, which the
    // index keeps as `unresolved.spec`; an example env file; a JWT in a test file.
    f.write(
        "src/app.ts",
        &format!(
            "import './postgres://{DB_USERINFO}/app';\nimport './{AWS_KEY}';\nimport './nope';\n"
        ),
    );
    f.write(".env.example", &format!("AWS_ACCESS_KEY_ID={AWS_KEY}\n"));
    f.write(
        "src/app.test.ts",
        &format!("import './{JWT}';\nexport const token = '{JWT}';\n"),
    );
    // A commit message with a pasted token.
    f.commit(&format!("wire the db, token {GH_TOKEN}"));
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let ix = Index::open_read(&w.index_path()).unwrap();

    // The control: the references are in the index, masked.
    let specs: Vec<String> = ix
        .conn
        .prepare("SELECT spec FROM unresolved ORDER BY spec")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(specs.iter().any(|s| s.contains("[REDACTED]")), "{specs:?}");
    assert!(specs.iter().any(|s| s == "./nope"), "{specs:?}");

    let planted = [AWS_KEY, JWT, DB_USERINFO, GH_TOKEN];
    let mut found = Vec::new();
    for (t, c) in text_columns(&ix) {
        for p in planted {
            let n: i64 = ix
                .conn
                .query_row(
                    &format!("SELECT count(*) FROM \"{t}\" WHERE instr(\"{c}\", ?1) > 0"),
                    [p],
                    |r| r.get(0),
                )
                .unwrap();
            if n > 0 {
                found.push(format!("{t}.{c} holds {p}"));
            }
        }
    }
    assert!(found.is_empty(), "{found:?}");
}
