//! IDX-22: unresolved imports as intervals, and the run's coverage totals.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::derive::coverage_report;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

#[test]
fn unresolved_imports_open_and_close_with_the_alias_that_fixes_them() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("tsconfig.json", r#"{"compilerOptions":{"baseUrl":"."}}"#);
    f.write(
        "src/app.ts",
        "import '#lib/util';\nimport './nope';\nimport 'left-pad';\n",
    );
    f.write("lib/util.ts", "export {};\n");
    f.commit("one");
    // Fix the alias in tsconfig.json: '#lib/util' resolves from here on.
    f.write(
        "tsconfig.json",
        r##"{"compilerOptions":{"baseUrl":".","paths":{"#lib/*":["lib/*"]}}}"##,
    );
    f.commit("alias");
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
        SyncOutcome::Synced(s) => assert_eq!(s.coverage, (2, 2, 3, 2)),
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let ix = Index::open_read(&w.index_path()).unwrap();
    // The golden unresolved list, as intervals.
    let rows: Vec<String> = ix
        .conn
        .prepare(
            "SELECT fp.path || ':' || u.line || ' ' || u.spec || ' ' || u.stage || '/' || u.reason
                    || ' ' || u.from_seq || '-' || coalesce(u.to_seq, 'open')
             FROM unresolved u JOIN file_path fp ON fp.file = u.file ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            "src/app.ts:1 #lib/util resolve/not-found 0-1",
            "src/app.ts:2 ./nope resolve/not-found 0-open",
        ]
    );
    let c = coverage_report(&ix.conn).unwrap().unwrap();
    assert_eq!((c.files_parsed, c.files_total), (2, 2));
    assert_eq!(
        c.unresolved,
        [("src/app.ts".to_owned(), 2, "./nope".to_owned())]
    );
}
