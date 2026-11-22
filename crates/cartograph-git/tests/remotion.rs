//! IDX-04's done-when check on a real history. Run with
//! `CARTOGRAPH_REMOTION=<clone> cargo test --release -p cartograph-git -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use cartograph_git::{ObjectId, Repo};

/// `git ls-tree -r`, regular files only: git's own answer, not gix's.
fn ls_tree(repo: &str, rev: &str) -> BTreeMap<String, String> {
    let out = Command::new("git")
        .args(["--git-dir", repo, "ls-tree", "-r", "-z", rev])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter_map(|l| {
            let (meta, path) = l.split_once('\t')?;
            let mut m = meta.split(' ');
            let (mode, kind, oid) = (m.next()?, m.next()?, m.next()?);
            (kind == "blob" && mode.starts_with("100")).then(|| (path.to_owned(), oid.to_owned()))
        })
        .collect()
}

#[test]
#[ignore = "needs CARTOGRAPH_REMOTION pointing at a remotion clone"]
fn remotion_walk_reproduces_ls_tree() {
    let path = PathBuf::from(std::env::var("CARTOGRAPH_REMOTION").expect("CARTOGRAPH_REMOTION"));
    let tip = std::env::var("CARTOGRAPH_REMOTION_TIP").unwrap_or_else(|_| "2b1b0c9".into());
    let repo = Repo::open(&path).unwrap();
    let to = repo.resolve(&tip).unwrap();
    let t = Instant::now();
    let steps: Vec<_> = repo
        .walk_first_parent(None, to, |_| true)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let walk_ms = t.elapsed().as_millis();
    let events: usize = steps.iter().map(|s| s.changes.len()).sum();
    let n = steps.len();
    let samples: Vec<usize> = (1..=24).map(|i| i * n / 24 - 1).collect();
    let mut tree: BTreeMap<String, ObjectId> = BTreeMap::new();
    let mut checked = 0;
    for (i, s) in steps.iter().enumerate() {
        for c in &s.changes {
            match c.new {
                Some(o) => tree.insert(c.path.clone(), o),
                None => tree.remove(&c.path),
            };
        }
        if samples.contains(&i) {
            let got: BTreeMap<String, String> = tree
                .iter()
                .map(|(p, o)| (p.clone(), o.to_string()))
                .collect();
            assert_eq!(
                got,
                ls_tree(path.to_str().unwrap(), &s.sha.to_string()),
                "step {i}"
            );
            checked += 1;
        }
    }
    println!(
        "steps {n}, changed regular-file events {events}, walk {walk_ms} ms; \
         tree equals git ls-tree -r at {checked} sampled commits (the last is head)"
    );
    if tip == "2b1b0c9" {
        assert_eq!(n, 9_498);
    }
}
