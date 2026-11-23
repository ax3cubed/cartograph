//! Golden indexes (HD-03): each fixture is generated, indexed with a fresh fact cache, and its fact
//! hash and fact list compared with `fixtures/golden/`. A change to an extractor, the replay or a
//! generator shows up here as a diff to review (`cargo insta review`), and CI fails until it is
//! accepted. The same golden on Linux, macOS and Windows is the cross-platform check.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use cartograph_fixtures::{acme, fact_hash, fact_list, history, index, layout, northwind};

fn golden(name: &str, text: &str) {
    insta::with_settings!({ snapshot_path => "../../golden", prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!(name, text);
    });
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
}

/// The fact hash and the fact list of one repository's index.
fn indexed(repo: &Path) -> String {
    let d = tempfile::tempdir().unwrap();
    let ix = index(repo, d.path());
    let list = fact_list(&ix);
    format!("{}\n\n{}\n", fact_hash(&ix), list.join("\n"))
}

#[test]
fn northwind_golden() {
    let repo = cartograph_fixtures::northwind();
    golden("northwind", &indexed(&repo));
}

#[test]
fn northwind_is_the_demo() {
    let repo = cartograph_fixtures::northwind();
    let demo = northwind::demo();
    assert_eq!(demo.services.len(), 6);
    assert_eq!(demo.commits.len(), 22);
    let log = git(&repo, &["log", "--reverse", "--format=%at %an %s"]);
    let want: Vec<String> = demo
        .commits
        .iter()
        .map(|c| format!("{} {} {}", northwind::noon(&c.date), c.author, c.message))
        .collect();
    assert_eq!(log.lines().collect::<Vec<_>>(), want);
    let top: BTreeSet<String> = git(&repo, &["ls-tree", "--name-only", "-d", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    let services: BTreeSet<String> = demo.services.iter().map(|s| s.name.clone()).collect();
    assert_eq!(top, services);
    let files = git(&repo, &["ls-tree", "-r", "--name-only", "HEAD"]);
    for (s, m, f) in &demo.files {
        let name = &demo.services.iter().find(|x| &x.id == s).unwrap().name;
        let p = format!("{name}/src/{m}/{f}");
        assert!(files.lines().any(|l| l == p), "{p} is in the repository");
    }
}

#[test]
fn history_golden() {
    golden("history", &indexed(&cartograph_fixtures::history()));
}

#[test]
fn history_has_every_case() {
    let repo = cartograph_fixtures::history();
    let commit = |msg: &str| git(&repo, &["log", "--format=%H", "-F", "--grep", msg, "main"]);
    let renames = |msg: &str| {
        let c = commit(msg);
        git(
            &repo,
            &["diff", "-M", "--name-status", &format!("{c}~1"), &c],
        )
    };
    assert!(renames(history::RENAME_100).contains("R100\tsrc/util.ts\tsrc/lib/util.ts"));
    assert!(renames(history::RENAME_92).contains("R092\tsrc/big.ts\tsrc/core/big.ts"));
    let low = renames(history::RENAME_UNDER_50);
    assert!(
        low.contains("D\tsrc/config.ts") && low.contains("A\tsrc/settings.ts"),
        "{low}"
    );

    let gone = git(
        &repo,
        &[
            "log",
            "--format=%s",
            "--name-status",
            "main",
            "--",
            "src/gone.ts",
        ],
    );
    assert!(
        gone.contains("D\tsrc/gone.ts") && gone.contains("re-add gone"),
        "{gone}"
    );

    let merge = commit("merge feature");
    assert_eq!(
        git(&repo, &["rev-list", "--parents", "-n1", &merge])
            .split(' ')
            .count(),
        3
    );
    let first_parent = git(&repo, &["rev-list", "--first-parent", "main"]);
    let feature = git(&repo, &["rev-parse", "feature"]);
    assert!(
        !first_parent.contains(&feature),
        "the feature is off the first-parent line"
    );

    for (gone_ref, why) in [
        ("topic-before-rebase", "rebased"),
        ("before-force-push", "force-pushed"),
    ] {
        let r = git(&repo, &["rev-parse", gone_ref]);
        let reachable = Command::new("git")
            .args(["-C"])
            .arg(&repo)
            .args(["merge-base", "--is-ancestor", &r, "main"])
            .status()
            .unwrap()
            .success();
        assert!(!reachable, "{gone_ref} was {why} away");
    }
    let topic = git(&repo, &["log", "--format=%s", "main", "-2", "--skip=1"]);
    assert_eq!(topic, "topic: topic uses gone\ntopic: add topic");
}

#[test]
fn layout_has_every_shape() {
    use cartograph_git::Repo;
    let root = cartograph_fixtures::layout();
    assert!(Repo::open(&root.join("shallow")).unwrap().is_shallow());
    assert!(Repo::open(&root.join("bare.git")).unwrap().is_bare());
    let wts = git(&root.join("monorepo"), &["worktree", "list", "--porcelain"]);
    for wt in layout::WORKTREES {
        Repo::open(&root.join(wt)).unwrap();
        assert!(wts.contains(wt), "{wts}");
    }
    assert!(
        !wts.contains("prunable"),
        "worktrees follow the folder: {wts}"
    );
    let sub = git(&root.join("with-submodule"), &["submodule", "status"]);
    assert!(
        sub.contains("vendor/go-services") && !sub.starts_with('-'),
        "{sub}"
    );
    assert!(Repo::open(&root.join("notes")).is_err());
    let ws = std::fs::read_to_string(root.join("layout.code-workspace")).unwrap();
    assert_eq!(ws, layout::CODE_WORKSPACE);
}

/// Board 00's numbers, generated in under 60 s (or found in the cache).
#[test]
fn acme_scale_has_board_00s_numbers() {
    let t = Instant::now();
    let root = cartograph_fixtures::acme_scale();
    let took = t.elapsed();
    assert!(took.as_secs() < 60, "acme-scale took {took:?}");
    let (mut commits, mut files) = (0, 0);
    for name in acme::repo_names() {
        let repo = root.join(&name);
        commits += git(&repo, &["rev-list", "--count", "HEAD"])
            .parse::<usize>()
            .unwrap();
        files += git(&repo, &["ls-tree", "-r", "--name-only", "HEAD"])
            .lines()
            .filter(|l| l.ends_with(".ts"))
            .count();
    }
    assert_eq!((commits, files), (acme::COMMITS, acme::FILES));
}

/// acme-scale's fact hash per repository. Ignored by default: indexing its 35,439 commits takes
/// about 16 s in a debug build (3 s in release). Run it with
/// `cargo test --release -p cartograph-fixtures --test golden -- --ignored`.
#[test]
#[ignore = "slow: indexes all of acme-scale"]
fn acme_scale_golden() {
    let root = cartograph_fixtures::acme_scale();
    let mut text = String::new();
    for name in acme::repo_names() {
        let d = tempfile::tempdir().unwrap();
        let ix = index(&root.join(&name), d.path());
        let list = fact_list(&ix);
        let count = |p: &str| list.iter().filter(|l| l.starts_with(p)).count();
        text.push_str(&format!(
            "{name} {} commits {} paths {} facts {} intervals {}\n",
            fact_hash(&ix),
            count("commit "),
            count("path "),
            count("fact "),
            count("link ")
        ));
    }
    golden("acme-scale", &text);
}
