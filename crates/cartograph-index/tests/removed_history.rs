//! A folder removed from the workspace keeps its history (IDX-14): a later sync that replays from
//! below its removal step (a changed exclude, a rewrite, a late fetch) must not drop its steps.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn count(w: &Workspace, sql: &str) -> i64 {
    let ix = Index::open_read(&w.index_path()).unwrap();
    ix.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn spec(f: &Fixture, name: &str) -> RepoSpec {
    RepoSpec {
        name: name.into(),
        path: f.dir.clone(),
        ..Default::default()
    }
}

#[test]
fn a_replay_from_the_start_keeps_a_removed_repo_s_history() {
    let d = tempfile::tempdir().unwrap();
    let mut api = Fixture::init(&d.path().join("api"));
    let mut web = Fixture::init(&d.path().join("web"));
    api.set_clock(1_600_000_000);
    api.write(
        "package.json",
        r#"{"name":"@acme/api","exports":"./src/index.ts"}"#,
    );
    api.write("src/index.ts", "export const x = 1;\n");
    api.write("dist/out.ts", "import '../src/index';\n");
    api.commit("api: first");
    web.set_clock(1_600_000_100);
    web.write(
        "src/app.ts",
        "import { x } from '@acme/api';\nimport './util';\n",
    );
    web.write("src/util.ts", "export const u = 1;\n");
    web.commit("web: first");
    web.set_clock(1_600_000_200);
    web.write("src/util.ts", "export const u = 2;\n");
    web.commit("web: second");
    api.set_clock(1_600_000_300);
    api.write("src/index.ts", "export const x = 2;\n");
    api.commit("api: second");

    let mut w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![spec(&api, "api"), spec(&web, "web")],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let web_commits =
        "SELECT count(*) FROM commits c JOIN repo r ON r.id = c.repo WHERE r.name = 'web'";
    let web_versions = "SELECT count(*) FROM file_version v JOIN file f ON f.id = v.file
                        JOIN repo r ON r.id = f.repo WHERE r.name = 'web'";
    let web_intervals = "SELECT count(*) FROM interval i JOIN link l ON l.id = i.link
                         JOIN node n ON n.id = l.src JOIN file f ON f.id = n.file
                         JOIN repo r ON r.id = f.repo WHERE r.name = 'web'";
    assert_eq!(count(&w, web_commits), 2);

    w.repos.retain(|r| r.name == "api");
    assert_eq!(run(&w, &mut cache).removed, ["web"]);
    let removed = (
        count(&w, web_commits),
        count(&w, web_versions),
        count(&w, web_intervals),
    );
    assert_eq!(removed.0, 3, "two commits and the removal step");

    // A changed exclude walks every current repo again from the start.
    w.exclude = vec!["**/dist".into()];
    let s = run(&w, &mut cache);
    assert_eq!(s.replayed_from, Some(-1));
    assert_eq!(
        (
            count(&w, web_commits),
            count(&w, web_versions),
            count(&w, web_intervals),
        ),
        removed,
        "web's history is still there"
    );
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM file_path WHERE path LIKE 'api/dist/%'"
        ),
        0,
        "the new exclude applies to the current repo"
    );
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM file_path WHERE path LIKE 'web/%' AND to_seq IS NULL"
        ),
        0,
        "web stays removed"
    );
    // And a further sync changes nothing.
    assert_eq!(run(&w, &mut cache).steps, 0);
    assert_eq!(count(&w, web_commits), removed.0);
}
