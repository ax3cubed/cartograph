//! IDX-19: rewritten history is replayed from the merge base, and the result equals a rebuild.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::{Index, fact_hash};
use proptest::prelude::*;

const T0: i64 = 1_600_000_000;

fn ws(dir: &Path, repos: &[&Fixture]) -> Workspace {
    Workspace {
        dir: dir.to_owned(),
        repos: repos
            .iter()
            .map(|f| RepoSpec {
                name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
                path: f.dir.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn hash(w: &Workspace) -> String {
    let ix = Index::open_read(&w.index_path()).unwrap();
    fact_hash(&ix.conn).unwrap().to_string()
}

/// Syncs `w` and checks the result equals a sync of the same repos into an empty folder.
fn equals_rebuild(w: &Workspace, repos: &[&Fixture], fresh: &Path, cache: &mut FactCache) {
    let rebuilt = ws(fresh, repos);
    run(&rebuilt, cache);
    assert_eq!(hash(w), hash(&rebuilt));
}

/// One commit of `f` at `t`: file `i % 9` rewritten with imports of two others, a package import
/// across repos, and now and then a move.
fn work(f: &mut Fixture, t: i64, i: usize, tag: &str) -> String {
    let n = i % 9;
    f.write(
        &format!("src/m{n}.ts"),
        &format!(
            "import './m{}';\nimport './m{}';\nimport '@acme/{}';\nexport const v = '{tag}{i}';\n",
            (n + 1) % 9,
            (n + 4) % 9,
            if i.is_multiple_of(2) { "a" } else { "b" }
        ),
    );
    if i % 13 == 12 {
        let from = f.dir.join(format!("src/m{n}.ts"));
        let body = std::fs::read_to_string(&from).unwrap();
        std::fs::remove_file(from).unwrap();
        f.write(&format!("src/moved/m{n}.ts"), &body);
    }
    f.set_clock(t);
    f.commit(&format!("{tag}{i}"))
}

fn repo(root: &Path, name: &str) -> Fixture {
    let mut f = Fixture::init(&root.join(name));
    f.write(
        "package.json",
        &format!(r#"{{"name":"@acme/{name}","main":"src/m0.ts"}}"#),
    );
    for n in 0..9 {
        f.write(&format!("src/m{n}.ts"), "export const v = 0;\n");
    }
    f.set_clock(T0);
    f.commit("first");
    f
}

/// The exit gate: a force-push that drops and rewrites the last 50 commits of one repo in a
/// three-repo workspace ends in the fact hash of a rebuild, with the rewrite recorded.
#[test]
fn a_force_push_of_50_commits_equals_a_rebuild() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        repo(d.path(), "a"),
        repo(d.path(), "b"),
        repo(d.path(), "c"),
    );
    let mut shas = Vec::new();
    for i in 0..80 {
        let t = T0 + 100 * (i as i64 + 1);
        shas.push(work(&mut a, t, i, "a"));
        work(&mut b, t + 10, i, "b");
        if i % 4 == 0 {
            work(&mut c, t + 20, i, "c");
        }
    }
    let w = ws(&d.path().join("ws"), &[&a, &b, &c]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let old_head = a.git(&["rev-parse", "HEAD"]);
    // Drop a's last 50 commits and write 50 different ones; b and c keep their later commits.
    let base = &shas[29];
    a.git(&["reset", "-q", "--hard", base]);
    for i in 30..80 {
        work(&mut a, T0 + 100 * (i as i64 + 1) + 50, i, "rewritten");
    }
    let s = run(&w, &mut cache);
    assert_eq!(s.rewritten, ["a"]);
    equals_rebuild(&w, &[&a, &b, &c], &d.path().join("fresh"), &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let (old, new, base_sha, at, archived): (String, String, String, i64, i64) = ix
        .conn
        .query_row(
            "SELECT lower(hex(old_head)), lower(hex(new_head)), lower(hex(base)), at, archived
             FROM rewrite",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        (old, new, &base_sha),
        (old_head, a.git(&["rev-parse", "HEAD"]), base)
    );
    // G04's dashed marker: rewrite.at is the base commit's place on the time axis.
    let base_time: i64 = ix
        .conn
        .query_row(
            "SELECT corrected_time FROM commits WHERE lower(hex(sha)) = ?1",
            [&base_sha],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, base_time);
    assert!(archived > 0, "the old history's links are kept");
    let gone: i64 = ix
        .conn
        .query_row(
            "SELECT count(*) FROM archived_interval x
             WHERE NOT EXISTS (SELECT 1 FROM commits c WHERE c.sha = x.from_sha)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(gone > 0, "keyed by the old history's shas");
}

/// Amend, rebase and a push of unrelated history, each in one repo while the other has later
/// commits.
#[test]
fn amend_rebase_and_unrelated_history_equal_a_rebuild() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, mut b) = (repo(d.path(), "a"), repo(d.path(), "b"));
    for i in 0..12 {
        work(&mut a, T0 + 100 * (i as i64 + 1), i, "a");
        work(&mut b, T0 + 100 * (i as i64 + 1) + 30, i, "b");
    }
    let w = ws(&d.path().join("ws"), &[&a, &b]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);

    // Amend: the last commit replaced.
    a.git(&["reset", "-q", "--hard", "HEAD~1"]);
    work(&mut a, T0 + 1_150, 11, "amended");
    assert_eq!(run(&w, &mut cache).rewritten, ["a"]);
    equals_rebuild(&w, &[&a, &b], &d.path().join("fresh1"), &mut cache);

    // Rebase: the last four commits replayed on a new commit, same changes, new shas.
    let tail: Vec<String> = a
        .git(&["rev-list", "--reverse", "HEAD~4..HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    a.git(&["reset", "-q", "--hard", "HEAD~4"]);
    a.write("src/extra.ts", "export const extra = 1;\n");
    a.set_clock(T0 + 1_200);
    a.commit("upstream");
    for (k, sha) in tail.iter().enumerate() {
        a.git(&["checkout", "-q", sha, "--", "."]);
        a.set_clock(T0 + 1_300 + k as i64);
        a.commit(&format!("rebased {k}"));
    }
    assert_eq!(run(&w, &mut cache).rewritten, ["a"]);
    equals_rebuild(&w, &[&a, &b], &d.path().join("fresh2"), &mut cache);

    // Unrelated history: no merge base, so the repo is walked again from its root.
    a.git(&["checkout", "-q", "--orphan", "other"]);
    a.git(&["rm", "-rq", "--cached", "."]);
    for e in std::fs::read_dir(a.dir.join("src")).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            std::fs::remove_file(p).unwrap();
        }
    }
    a.write("src/fresh.ts", "import './m1';\n");
    a.write("src/m1.ts", "export {};\n");
    a.set_clock(T0 + 2_000);
    let sha = a.commit("unrelated root");
    a.git(&["update-ref", "refs/heads/main", &sha]);
    a.git(&["checkout", "-q", "main"]);
    assert_eq!(run(&w, &mut cache).rewritten, ["a"]);
    equals_rebuild(&w, &[&a, &b], &d.path().join("fresh3"), &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let rewrites: Vec<Option<Vec<u8>>> = ix
        .conn
        .prepare("SELECT base FROM rewrite ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rewrites.len(), 3);
    assert!(rewrites[2].is_none(), "no merge base");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 6, ..ProptestConfig::default() })]

    /// Rewrite then sync equals a rebuild, wherever the rewrite starts and however long it is.
    #[test]
    fn rewrite_then_sync_equals_rebuild(len in 3usize..14, keep in 0usize..10, more in 1usize..8) {
        let d = tempfile::tempdir().unwrap();
        let (mut a, mut b) = (repo(d.path(), "a"), repo(d.path(), "b"));
        let mut shas = Vec::new();
        for i in 0..len {
            shas.push(work(&mut a, T0 + 100 * (i as i64 + 1), i, "a"));
            work(&mut b, T0 + 100 * (i as i64 + 1) + 40, i, "b");
        }
        let w = ws(&d.path().join("ws"), &[&a, &b]);
        let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
        run(&w, &mut cache);
        let keep = keep.min(len - 1);
        a.git(&["reset", "-q", "--hard", &shas[keep]]);
        for i in 0..more {
            work(&mut a, T0 + 100 * (keep as i64 + 1) + 7 + i as i64, i + 100, "r");
        }
        run(&w, &mut cache);
        let rebuilt = ws(&d.path().join("fresh"), &[&a, &b]);
        run(&rebuilt, &mut cache);
        prop_assert_eq!(hash(&w), hash(&rebuilt));
    }
}
