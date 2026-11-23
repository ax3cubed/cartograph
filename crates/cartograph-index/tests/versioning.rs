//! IDX-21: an extractor's version bump re-extracts only its own blobs and replays what changed; an
//! unchanged run rebuilds no derived row.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_parse::BlobFacts;
use cartograph_store::rusqlite::Connection;
use cartograph_store::{Index, fact_hash};

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn ws(f: &Fixture, dir: &Path) -> Workspace {
    Workspace {
        dir: dir.to_owned(),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn rows(c: &Connection, sql: &str) -> Vec<String> {
    c.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_ts_imports_bump_reparses_only_ts_and_leaves_go_alone() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("go.mod", "module example.com/m\n");
    f.write("a/a.go", "package a\nimport \"example.com/m/b\"\n");
    f.write("b/b.go", "package b\n");
    f.write("web/x.ts", "import './y';\n");
    f.write("web/y.ts", "export {};\n");
    f.commit("one");
    f.write("web/x.ts", "import './y';\nimport './z';\n");
    f.write("web/z.js", "module.exports = 1;\n");
    f.commit("two");
    let w = ws(&f, &d.path().join("ws"));
    let cache_path = d.path().join("facts.db");
    let mut cache = FactCache::open(&cache_path).unwrap();
    run(&w, &mut cache);
    // An unchanged run rebuilds no derived row.
    assert_eq!(run(&w, &mut cache).derived.rebuilt(), 0);

    // The index was written by a ts-imports 0.9.0 whose facts were different (none), and the cache
    // holds no ts-imports 1.0.0 rows: what a version bump looks like to this binary.
    let go_facts = "SELECT hex(f.blob) || e.name || e.version || hex(f.payload) FROM fact f
                    JOIN extractor e ON e.id = f.extractor WHERE e.name NOT LIKE 'ts-imports/%' ORDER BY 1";
    let go_cache = "SELECT hex(blob) || extractor || version || hex(payload) FROM fact
                    WHERE extractor NOT LIKE 'ts-imports/%' ORDER BY 1";
    let (index_before, cache_before) = {
        let ix = Connection::open(w.index_path()).unwrap();
        ix.execute(
            "UPDATE fact SET payload = ?1 WHERE extractor IN
               (SELECT id FROM extractor WHERE name LIKE 'ts-imports/%')",
            [BlobFacts::Facts(vec![]).encode()],
        )
        .unwrap();
        ix.execute(
            "UPDATE extractor SET version = '0.9.0' WHERE name LIKE 'ts-imports/%'",
            [],
        )
        .unwrap();
        let c = Connection::open(&cache_path).unwrap();
        c.execute("DELETE FROM fact WHERE extractor LIKE 'ts-imports/%'", [])
            .unwrap();
        (rows(&ix, go_facts), rows(&c, go_cache))
    };
    let s = run(&w, &mut cache);
    assert_eq!(s.extract.parsed, 4, "the four TS/JS blobs, nothing else");
    assert_eq!(
        s.replayed_from,
        Some(-1),
        "x.ts's first blob was alive from the start"
    );
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(rows(&ix.conn, go_facts), index_before, "Go facts untouched");
    let c = Connection::open(&cache_path).unwrap();
    assert_eq!(rows(&c, go_cache), cache_before, "Go cache rows untouched");
    assert_eq!(
        rows(
            &ix.conn,
            "SELECT DISTINCT version FROM extractor WHERE name LIKE 'ts-imports/%'"
        ),
        ["1.0.0"]
    );
    // The same facts and intervals as a fresh index.
    let fresh = ws(&f, &d.path().join("fresh"));
    run(&fresh, &mut cache);
    let fresh_ix = Index::open_read(&fresh.index_path()).unwrap();
    assert_eq!(
        fact_hash(&ix.conn).unwrap().to_string(),
        fact_hash(&fresh_ix.conn).unwrap().to_string()
    );
}
