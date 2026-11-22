use super::*;
use crate::fixture::Fixture;
use proptest::prelude::*;

fn all(_: &str) -> bool {
    true
}

/// Applies steps to a path -> blob map, as the replay does.
fn replay(tree: &mut BTreeMap<String, ObjectId>, step: &CommitStep) {
    for c in &step.changes {
        assert_eq!(tree.get(&c.path), c.old.as_ref(), "old side of {}", c.path);
        match c.new {
            Some(n) => tree.insert(c.path.clone(), n),
            None => tree.remove(&c.path),
        };
    }
}

fn as_hex(t: &BTreeMap<String, ObjectId>) -> BTreeMap<String, String> {
    t.iter().map(|(p, o)| (p.clone(), o.to_string())).collect()
}

/// bench.mjs's self-check history, ported, plus a merge, a symlink and a clock that goes backwards.
fn hand_made(dir: &Path) -> (Fixture, Vec<String>) {
    let mut f = Fixture::init(dir);
    let mut shas = Vec::new();
    f.write("src/a.ts", "import a from './b';\nimport l from 'lib';\n");
    shas.push(f.commit("add a"));
    f.write("src/b/index.ts", "export const b = 1;\n");
    f.write("lib/package.json", "{\"name\":\"lib\"}\n");
    shas.push(f.commit("add b and lib"));
    f.write("src/b.ts", "export const b = 1;\n");
    shas.push(f.commit("a better candidate"));
    // A side branch, merged: the merge is diffed against its first parent only.
    let main = shas.last().unwrap().clone();
    f.write("side.ts", "export {};\n");
    let side = f.commit("side work");
    f.git(&["update-ref", "HEAD", &main]);
    f.git(&["read-tree", "--reset", "-u", &main]);
    f.write("side.ts", "export {};\n");
    f.write("lib/package.json", "{\"name\":\"other\"}\n");
    f.write("src/a.ts", "import a from './b';\n");
    f.set_clock(1_500_000_000); // committer clock goes backwards
    shas.push(f.commit_with_parents("merge side; rename package", &[&main, &side]));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("src/b.ts", dir.join("link.ts")).unwrap();
        f.remove("src/b/index.ts");
        shas.push(f.commit("symlink and delete"));
    }
    (f, shas)
}

#[test]
#[cfg(unix)] // the history has a symlink
fn hand_made_history_golden_steps() {
    let d = tempfile::tempdir().unwrap();
    let (f, shas) = hand_made(d.path());
    let repo = Repo::open(d.path()).unwrap();
    let steps: Vec<CommitStep> = repo
        .walk_first_parent(None, repo.head().unwrap(), all)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        steps.iter().map(|s| s.sha.to_string()).collect::<Vec<_>>(),
        shas,
        "first parent only, oldest first"
    );
    // Corrected times strictly increase although the committer clock went backwards.
    assert!(
        steps
            .windows(2)
            .all(|w| w[1].corrected_time > w[0].corrected_time)
    );
    assert!(steps.iter().any(|s| s.time < s.corrected_time));
    let mut tree = BTreeMap::new();
    let mut golden = String::new();
    for (i, s) in steps.iter().enumerate() {
        replay(&mut tree, s);
        assert_eq!(as_hex(&tree), f.ls_tree(&shas[i]), "tree at step {i}");
        golden += &format!("{i} {} by {}\n", s.summary, s.author);
        for c in &s.changes {
            let short =
                |o: &Option<ObjectId>| o.map_or("-".into(), |o| o.to_string()[..7].to_owned());
            golden += &format!("  {} {} -> {}\n", c.path, short(&c.old), short(&c.new));
        }
    }
    insta::assert_snapshot!(golden);
}

#[test]
fn incremental_walk_continues_where_the_first_stopped() {
    let d = tempfile::tempdir().unwrap();
    let (_f, shas) = hand_made(d.path());
    let repo = Repo::open(d.path()).unwrap();
    let head = repo.head().unwrap();
    let full: Vec<_> = repo
        .walk_first_parent(None, head, all)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let from = ObjectId::from_hex(shas[1].as_bytes()).unwrap();
    let rest: Vec<_> = repo
        .walk_first_parent(Some(from), head, all)
        .unwrap()
        .with_prev_corrected(full[1].corrected_time)
        .map(Result::unwrap)
        .collect();
    assert_eq!(rest, full[2..]);
    let stranger = ObjectId::from_hex(b"0123456789012345678901234567890123456789").unwrap();
    assert!(matches!(
        repo.walk_first_parent(Some(stranger), head, all),
        Err(GitError::NotAncestor { .. })
    ));
}

#[test]
fn filter_applies_inside_the_walk() {
    let d = tempfile::tempdir().unwrap();
    let (f, _) = hand_made(d.path());
    let repo = Repo::open(d.path()).unwrap();
    let keep = |p: &str| p.ends_with(".json");
    let mut tree = BTreeMap::new();
    for s in repo
        .walk_first_parent(None, repo.head().unwrap(), keep)
        .unwrap()
    {
        replay(&mut tree, &s.unwrap());
    }
    let want: BTreeMap<_, _> = f
        .ls_tree("HEAD")
        .into_iter()
        .filter(|(p, _)| keep(p))
        .collect();
    assert_eq!(as_hex(&tree), want);
    assert_eq!(
        as_hex(&repo.tree_at(repo.head().unwrap(), keep).unwrap()),
        want
    );
}

#[test]
fn read_blob_refuses_oversized_blobs_from_the_header() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(d.path());
    f.write("big.ts", &"x".repeat(5000));
    f.commit("big");
    let repo = Repo::open(d.path()).unwrap();
    let oid = repo.tree_at(repo.head().unwrap(), all).unwrap()["big.ts"];
    let local = repo.local();
    assert_eq!(read_blob(&local, oid, 5000).unwrap().len(), 5000);
    assert!(matches!(
        read_blob(&local, oid, 4999),
        Err(GitError::TooLarge { size: 5000, .. })
    ));
    assert!(!repo.is_shallow());
    assert!(!repo.is_bare());
}

#[derive(Debug, Clone)]
enum Op {
    Write(u8, u8),
    Delete(u8),
    Commit,
    Branch,
    Merge,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0u8..12, 0u8..4).prop_map(|(p, c)| Op::Write(p, c)),
        2 => (0u8..12).prop_map(Op::Delete),
        2 => Just(Op::Commit),
        1 => Just(Op::Branch),
        1 => Just(Op::Merge),
    ]
}

const PATHS: [&str; 12] = [
    "a.ts",
    "b.ts",
    "src/c.ts",
    "src/d.tsx",
    "src/x/e.js",
    "src/x/f.mjs",
    "pkg/package.json",
    "pkg/src/index.ts",
    "pkg/src/deep/g.ts",
    "h.cjs",
    "src/x",
    "src/x/y/z.ts",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]
    /// Replaying the steps reproduces `git ls-tree -r` at every first-parent commit, on random
    /// histories with merges (whose second parent's tree differs from the first parent's).
    #[test]
    fn replayed_steps_reproduce_ls_tree(ops in proptest::collection::vec(op(), 1..40)) {
        let d = tempfile::tempdir().unwrap();
        let mut f = Fixture::init(d.path());
        f.write("root.ts", "");
        let mut main = vec![f.commit("root")];
        let mut side: Option<String> = None;
        for o in ops {
            match o {
                Op::Write(p, c) => {
                    // "src/x" as a file collides with the folder; git replaces one with the other.
                    let path = PATHS[p as usize];
                    let _ = std::fs::remove_dir_all(d.path().join(path));
                    for a in Path::new(path).ancestors().skip(1) {
                        let a = d.path().join(a);
                        if a.is_file() { std::fs::remove_file(&a).unwrap(); }
                    }
                    f.write(path, &format!("v{c}"));
                }
                Op::Delete(p) => {
                    let path = d.path().join(PATHS[p as usize]);
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_dir_all(&path);
                }
                Op::Commit => main.push(f.commit("c")),
                Op::Branch => {
                    let m = main.last().unwrap().clone();
                    side = Some(f.commit_with_parents("side", &[&m]));
                    f.git(&["update-ref", "HEAD", &m]);
                }
                Op::Merge => if let Some(s) = side.take() {
                    let m = main.last().unwrap().clone();
                    main.push(f.commit_with_parents("merge", &[&m, &s]));
                },
            }
        }
        main.push(f.commit_with_parents("tip", &[main.last().unwrap()]));
        let repo = Repo::open(d.path()).unwrap();
        let tip = ObjectId::from_hex(main.last().unwrap().as_bytes()).unwrap();
        let mut tree = BTreeMap::new();
        for (s, sha) in repo.walk_first_parent(None, tip, all).unwrap().zip(&main) {
            let s = s.unwrap();
            prop_assert_eq!(s.sha.to_string(), sha.clone());
            replay(&mut tree, &s);
            prop_assert_eq!(as_hex(&tree), f.ls_tree(sha));
        }
    }
}
