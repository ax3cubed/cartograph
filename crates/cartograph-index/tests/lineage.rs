//! IDX-18: rename detection and file lineage, over fixture repositories made with git.

use std::collections::BTreeSet;
use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::Repo;
use cartograph_git::fixture::Fixture;
use cartograph_index::build_single;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::{Index, fact_hash};
use proptest::prelude::*;

fn build(f: &Fixture, dir: &Path) -> Index {
    let out = dir.join("index.cartograph");
    let mut cache = FactCache::open(&dir.join("facts.db")).unwrap();
    build_single(&f.dir, None, &out, &mut cache, 2).unwrap();
    Index::open_read(&out).unwrap()
}

fn rows(ix: &Index, sql: &str) -> Vec<String> {
    ix.conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const BODY: &str = "export const a = 1;\nexport const b = 2;\nexport const c = 3;\nexport const d = 4;\n\
                    export const e = 5;\nexport const f = 6;\nexport const g = 7;\nexport const h = 8;\n";

/// A history with an exact move, an edited move, a swap of two files and a move plus a new file at
/// the old path.
fn moves(root: &Path) -> Fixture {
    let mut f = Fixture::init(root);
    f.write("src/util.ts", BODY);
    f.write("src/edit.ts", BODY);
    f.write("src/x.ts", "export const x = 'x one';\n");
    f.write("src/y.ts", "export const y = 'y one, a different file';\n");
    f.write("src/old.ts", BODY);
    f.write(
        "src/app.ts",
        "import './util';\nimport './edit';\nimport './old';\n",
    );
    f.commit("first");
    // An exact move, with the importer updated in the same commit.
    f.remove("src/util.ts");
    f.write("src/lib/util.ts", BODY);
    f.write(
        "src/app.ts",
        "import './lib/util';\nimport './edit';\nimport './old';\n",
    );
    f.commit("move util");
    // An edited move (one line of eight changed).
    f.remove("src/edit.ts");
    f.write(
        "src/lib/edit.ts",
        &BODY.replace("const h = 8", "const h = 9"),
    );
    f.write(
        "src/app.ts",
        "import './lib/util';\nimport './lib/edit';\nimport './old';\n",
    );
    f.commit("move and edit");
    // A swap: each path now holds the other's content.
    f.write("src/x.ts", "export const y = 'y one, a different file';\n");
    f.write("src/y.ts", "export const x = 'x one';\n");
    f.commit("swap");
    // A move plus a new file at the old path.
    f.remove("src/old.ts");
    f.write("src/moved.ts", BODY);
    f.write("src/old.ts", "export const fresh = true;\n");
    f.commit("move, and a new file where it was");
    f
}

#[test]
fn moves_keep_one_lineage_and_their_links() {
    let d = tempfile::tempdir().unwrap();
    let f = moves(&d.path().join("repo"));
    let ix = build(&f, d.path());
    // The golden lineage: each file's paths, in order, with how each path was reached.
    let lineage = rows(
        &ix,
        "SELECT group_concat(path || ' (' || via || coalesce(' ' || similarity || '%', '') || ')',
                             ' -> ')
         FROM (SELECT * FROM file_path ORDER BY file, from_seq) GROUP BY file ORDER BY 1",
    );
    assert_eq!(
        lineage,
        [
            "src/app.ts (add)",
            "src/edit.ts (add) -> src/lib/edit.ts (rename-similar 87%)",
            // Without break detection (git's -B), a path that still exists is a modification:
            // git and gix both read this as an edit of old.ts and a new moved.ts.
            "src/moved.ts (add)",
            "src/old.ts (add)",
            "src/util.ts (add) -> src/lib/util.ts (rename-exact)",
            "src/x.ts (add)",
            "src/y.ts (add)",
        ]
    );
    // A moved file is one file row with two file_path rows.
    let n: Vec<String> = rows(
        &ix,
        "SELECT count(*) || '' FROM file_path fp WHERE fp.file =
           (SELECT file FROM file_path WHERE path = 'src/lib/util.ts')",
    );
    assert_eq!(n, ["2"]);
    // The importer was updated in the commit that moved its target: its link never closed.
    let links = rows(
        &ix,
        "SELECT (SELECT path FROM file_path WHERE file = nd.file ORDER BY from_seq DESC LIMIT 1)
                || ' ' || i.from_seq || '-' || coalesce(i.to_seq, 'open')
         FROM interval i JOIN link l ON l.id = i.link JOIN node nd ON nd.id = l.dst
         ORDER BY 1",
    );
    assert_eq!(
        links,
        [
            "src/lib/edit.ts 0-open",
            "src/lib/util.ts 0-open",
            "src/old.ts 0-open"
        ]
    );
    // Board G10's history line comes from file_path alone.
    let g10 = rows(
        &ix,
        "SELECT 'Moved from ' || p.path || ', ' || coalesce(n.similarity, 100) || '% same content'
         FROM file_path n JOIN file_path p ON p.file = n.file AND p.to_seq = n.from_seq
         WHERE n.path = 'src/lib/edit.ts'",
    );
    assert_eq!(g10, ["Moved from src/edit.ts, 87% same content"]);
}

/// `git log -M50% --first-parent`'s renames, per commit, as (from, to).
fn git_renames(f: &Fixture) -> Vec<BTreeSet<(String, String)>> {
    let out = f.git(&[
        "log",
        "--first-parent",
        "--reverse",
        "-M50%",
        "--name-status",
        "--format=@%H",
    ]);
    let mut v: Vec<BTreeSet<(String, String)>> = Vec::new();
    for line in out.lines() {
        if line.starts_with('@') {
            v.push(BTreeSet::new());
        } else if line.starts_with('R') {
            let p: Vec<&str> = line.split('\t').collect();
            v.last_mut()
                .unwrap()
                .insert((p[1].to_owned(), p[2].to_owned()));
        }
    }
    v
}

#[test]
fn renames_agree_with_git_log_m50() {
    let d = tempfile::tempdir().unwrap();
    let f = moves(&d.path().join("repo"));
    let repo = Repo::open(&f.dir).unwrap();
    let ours: Vec<BTreeSet<(String, String)>> = repo
        .walk_first_parent(None, repo.head().unwrap(), |_| true)
        .unwrap()
        .map(|s| {
            s.unwrap()
                .renames
                .into_iter()
                .map(|r| (r.from, r.to))
                .collect()
        })
        .collect();
    assert_eq!(ours, git_renames(&f));
}

/// IDX-18 security: a commit with more rename candidates than the limit is noted, not stalled.
#[test]
fn a_huge_drop_hits_the_limit_and_is_noted() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    for i in 0..1_100 {
        f.write(
            &format!("a/{i}.ts"),
            &format!("export const v{i} = {i};\n// a\n// b\n"),
        );
    }
    f.commit("first");
    for i in 0..1_100 {
        f.remove(&format!("a/{i}.ts"));
        f.write(
            &format!("b/{i}.ts"),
            &format!("export const v{i} = {i};\n// a\n// c\n"),
        );
    }
    f.commit("vendored drop");
    let ix = build(&f, d.path());
    assert_eq!(rows(&ix, "SELECT kind FROM note"), ["rename-limit"]);
    assert_eq!(
        rows(
            &ix,
            "SELECT count(*) || '' FROM file_path WHERE via != 'add'"
        ),
        ["0"]
    );
}

/// The workspace sync gives the same lineages as a build, in one run or many.
#[test]
fn sync_and_build_agree_on_moves() {
    let d = tempfile::tempdir().unwrap();
    let f = moves(&d.path().join("repo"));
    let built = build(&f, d.path());
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let shas: Vec<String> = f
        .git(&["rev-list", "--reverse", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    let mut cache = FactCache::open(&d.path().join("facts2.db")).unwrap();
    for sha in &shas {
        f.git(&["update-ref", "refs/heads/main", sha]);
        match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
            SyncOutcome::Synced(_) => {}
            SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
        }
    }
    let synced = Index::open_read(&w.index_path()).unwrap();
    // The build names its repository after the folder; the hash keys repos by root commit only.
    assert_eq!(
        fact_hash(&synced.conn).unwrap().to_string(),
        fact_hash(&built.conn).unwrap().to_string()
    );
}

#[derive(Debug, Clone)]
enum Op {
    Add(u8),
    Edit(u8),
    Delete(u8),
    Move(u8, u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..8).prop_map(Op::Add),
        (0u8..8).prop_map(Op::Edit),
        (0u8..8).prop_map(Op::Delete),
        (0u8..8, 0u8..8).prop_map(|(a, b)| Op::Move(a, b)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    /// A lineage never holds two files alive at once: its path rows never overlap.
    #[test]
    fn lineages_never_merge_two_live_files(commits in proptest::collection::vec(
        proptest::collection::vec(op(), 1..4), 2..7)) {
        let d = tempfile::tempdir().unwrap();
        let mut f = Fixture::init(&d.path().join("repo"));
        let path = |i: u8| format!("src/f{i}.ts");
        let mut live: BTreeSet<u8> = BTreeSet::new();
        let mut n = 0u32;
        for ops in &commits {
            for op in ops {
                n += 1;
                match *op {
                    Op::Add(i) | Op::Edit(i) => {
                        f.write(&path(i), &format!("{BODY}// {i} {n}\n"));
                        live.insert(i);
                    }
                    Op::Delete(i) => {
                        f.remove(&path(i));
                        live.remove(&i);
                    }
                    Op::Move(a, b) if live.contains(&a) && a != b => {
                        let body = std::fs::read_to_string(f.dir.join(path(a))).unwrap();
                        f.remove(&path(a));
                        f.write(&path(b), &body);
                        live.remove(&a);
                        live.insert(b);
                    }
                    Op::Move(..) => {}
                }
            }
            if live.is_empty() {
                f.write(&path(0), BODY);
                live.insert(0);
            }
            f.commit("step");
        }
        let ix = build(&f, d.path());
        let overlaps: i64 = ix.conn.query_row(
            "SELECT count(*) FROM file_path a JOIN file_path b ON a.file = b.file AND a.rowid < b.rowid
             WHERE a.from_seq < coalesce(b.to_seq, 1 << 62) AND b.from_seq < coalesce(a.to_seq, 1 << 62)",
            [], |r| r.get(0)).unwrap();
        prop_assert_eq!(overlaps, 0);
        let live_paths: i64 = ix.conn.query_row(
            "SELECT count(*) FROM file_path WHERE to_seq IS NULL", [], |r| r.get(0)).unwrap();
        prop_assert_eq!(live_paths as usize, live.len());
    }
}
