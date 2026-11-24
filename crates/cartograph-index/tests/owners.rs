//! XS-23: owners from CODEOWNERS over history, kept current by incremental syncs.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn one(f: &Fixture, dir: &Path) -> Workspace {
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

/// `path owner from-to` per ownership row, by the lineage's last path.
fn rows(w: &Workspace) -> Vec<String> {
    let ix = Index::open_read(&w.index_path()).unwrap();
    let mut st = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = o.file
                     ORDER BY from_seq DESC LIMIT 1),
                    o.owner, o.from_seq, coalesce(CAST(o.to_seq AS TEXT), 'open')
             FROM ownership o ORDER BY 1, o.from_seq, o.owner",
        )
        .unwrap();
    st.query_map([], |r| {
        Ok(format!(
            "{} {} {}-{}",
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn owners_change_from_the_commit_that_changes_codeowners_and_survive_renames() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("src/a.ts", "export const a = 1;\n");
    f.write("lib/b.ts", "export const b = 1;\n");
    f.write(".github/CODEOWNERS", "* @everyone\n");
    f.commit("one");
    let w = one(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    assert_eq!(
        rows(&w),
        ["lib/b.ts @everyone 0-open", "src/a.ts @everyone 0-open"]
    );

    // An incremental sync: CODEOWNERS changes at step 1, and a file moves at step 2.
    f.write(
        ".github/CODEOWNERS",
        "* @everyone\n/src/ @acme/core @lead\n",
    );
    f.commit("two");
    f.git(&["mv", "src/a.ts", "lib/a.ts"]);
    f.commit("three");
    run(&w, &mut cache);
    assert_eq!(
        rows(&w),
        [
            "lib/a.ts @everyone 0-1",
            "lib/a.ts @acme/core 1-2",
            "lib/a.ts @lead 1-2",
            "lib/a.ts @everyone 2-open",
            "lib/b.ts @everyone 0-open",
        ]
    );

    // A file named only by an owner-less line is un-owned from the commit that adds the line,
    // and deleting CODEOWNERS closes every interval.
    f.write(".github/CODEOWNERS", "* @everyone\n/lib/b.ts\n");
    f.commit("four");
    f.git(&["rm", "-q", ".github/CODEOWNERS"]);
    f.commit("five");
    run(&w, &mut cache);
    assert_eq!(
        rows(&w),
        [
            "lib/a.ts @everyone 0-1",
            "lib/a.ts @acme/core 1-2",
            "lib/a.ts @lead 1-2",
            "lib/a.ts @everyone 2-4",
            "lib/b.ts @everyone 0-3",
        ]
    );
}

#[test]
fn a_hostile_codeowners_yields_no_markup_and_a_missing_one_yields_nothing() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "export {};\n");
    f.write(
        "CODEOWNERS",
        "* <img src=x onerror=alert(1)>\n/a.ts @ok\n/b.ts @x\u{7}y\n",
    );
    f.commit("one");
    let w = one(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    assert_eq!(rows(&w), ["a.ts @ok 0-open"]);

    let d2 = tempfile::tempdir().unwrap();
    let mut g = Fixture::init(&d2.path().join("repo"));
    g.write("a.ts", "export {};\n");
    g.commit("one");
    let w2 = one(&g, &d2.path().join("ws"));
    run(
        &w2,
        &mut FactCache::open(&d2.path().join("facts.db")).unwrap(),
    );
    assert!(rows(&w2).is_empty());
}
