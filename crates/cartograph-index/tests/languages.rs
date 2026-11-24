//! IDX-23: Go, Python and PHP onto one timeline, and the replay against a from-scratch index.
//!
//! The per-language public-repo oracle is an ignored test:
//! `CARTOGRAPH_ORACLE_REPO=<clone> cargo test --release -p cartograph-index --test languages -- --ignored --nocapture`.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use cartograph_facts::{FactCache, Replay, from_scratch};
use cartograph_git::Repo;
use cartograph_git::fixture::Fixture;
use cartograph_index::build::{changes_of, walk_and_extract};
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_parse::extractor_for;
use cartograph_store::Index;

/// One repository with Go, Python and PHP that changes over a few commits: modules move, a package
/// appears, an import breaks and is fixed.
fn mixed(root: &Path) -> Fixture {
    let mut f = Fixture::init(root);
    f.write("go.mod", "module example.com/app\n\ngo 1.22\nreplace example.com/vendored => ./third_party/vendored\n");
    f.write(
        "cmd/main.go",
        "package main\nimport (\n  \"fmt\"\n  \"example.com/app/internal/db\"\n  \"example.com/vendored/v\"\n  \"github.com/x/y\"\n)\n",
    );
    f.write("internal/db/db.go", "package db\n");
    f.write("third_party/vendored/v/v.go", "package v\n");
    f.write(
        "pyproject.toml",
        "[project]\nname = \"app\"\n[tool.setuptools.packages.find]\nwhere = [\"py\"]\n",
    );
    f.write("py/billing/__init__.py", "");
    f.write(
        "py/billing/core.py",
        "import os\nfrom . import models\nfrom .models import Invoice\nfrom billing.util import fmt\nimport missing_pkg\n",
    );
    f.write("py/billing/models.py", "class Invoice: pass\n");
    f.write(
        "composer.json",
        r#"{"name":"acme/app","autoload":{"psr-4":{"App\\":"php/src/"}}}"#,
    );
    f.write(
        "php/src/Http/Kernel.php",
        "<?php\nnamespace App\\Http;\nuse App\\Models\\User;\nuse Vendor\\Lib\\Thing;\nrequire_once __DIR__ . '/../bootstrap.php';\n",
    );
    f.write(
        "php/src/Models/User.php",
        "<?php\nnamespace App\\Models;\nclass User {}\n",
    );
    f.write("php/src/bootstrap.php", "<?php\n");
    f.commit("one");
    // billing.util appears; the Go package moves; User moves.
    f.write("py/billing/util.py", "def fmt(): pass\n");
    std::fs::rename(f.dir.join("internal/db"), f.dir.join("internal/store")).unwrap();
    f.write(
        "cmd/main.go",
        "package main\nimport (\n  \"fmt\"\n  \"example.com/app/internal/store\"\n  \"example.com/vendored/v\"\n)\n",
    );
    f.write(
        "php/src/Models/Account.php",
        "<?php\nnamespace App\\Models;\nclass Account {}\n",
    );
    f.remove("php/src/Models/User.php");
    f.commit("two");
    // The PSR-4 root moves: User resolves again under the new folder.
    f.write(
        "composer.json",
        r#"{"name":"acme/app","autoload":{"psr-4":{"App\\":"php/lib/"}}}"#,
    );
    f.write(
        "php/lib/Models/User.php",
        "<?php\nnamespace App\\Models;\nclass User {}\n",
    );
    f.write("go.mod", "module example.com/app\n\ngo 1.22\n");
    f.commit("three");
    f
}

/// Replays `repo`'s first-parent history and checks the links after every step against a
/// from-scratch index of the tree at that commit. Returns the links at head.
fn replay_equals_from_scratch(
    path: &Path,
    cache: &mut FactCache,
    every: usize,
) -> BTreeSet<cartograph_facts::Link> {
    let repo = Repo::open(path).unwrap();
    let (steps, facts, ..) = walk_and_extract(&repo, repo.head().unwrap(), cache, 4).unwrap();
    let mut replay = Replay::default();
    let n = steps.len();
    for (i, step) in steps.iter().enumerate() {
        replay.apply(&changes_of(step, &facts), i as u64);
        if i % every == 0 || i + 1 == n {
            let tree = repo.tree_at(step.sha, cartograph_parse::tracked).unwrap();
            let direct: HashMap<String, Arc<_>> = tree
                .iter()
                .map(|(p, o)| {
                    (
                        p.clone(),
                        Arc::clone(&facts[&(*o, extractor_for(p).unwrap().1)]),
                    )
                })
                .collect();
            assert_eq!(
                replay.links(),
                from_scratch(&direct),
                "step {i} ({})",
                step.sha
            );
        }
    }
    replay.links()
}

#[test]
fn go_python_and_php_replay_equals_a_from_scratch_index() {
    let d = tempfile::tempdir().unwrap();
    let f = mixed(&d.path().join("repo"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let links: Vec<String> = replay_equals_from_scratch(&f.dir, &mut cache, 1)
        .into_iter()
        .map(|(k, a, b)| format!("{} {a} -> {b}", k.as_str()))
        .collect();
    assert_eq!(
        links,
        [
            "import cmd/main.go -> internal/store/",
            "import php/src/Http/Kernel.php -> php/lib/Models/User.php",
            "import php/src/Http/Kernel.php -> php/src/bootstrap.php",
            "import py/billing/core.py -> py/billing/models.py",
            "import py/billing/core.py -> py/billing/util.py",
        ]
    );
}

#[test]
fn a_mixed_language_workspace_indexes_onto_one_timeline() {
    let d = tempfile::tempdir().unwrap();
    let mut shared = Fixture::init(&d.path().join("shared"));
    shared.write("go.mod", "module example.com/shared\n");
    shared.write("util/u.go", "package util\n");
    shared.write("pyproject.toml", "[project]\nname = \"acme-lib\"\n");
    shared.write("acme_lib/__init__.py", "");
    shared.write(
        "composer.json",
        r#"{"name":"acme/lib","autoload":{"psr-4":{"Acme\\Lib\\":"src/"}}}"#,
    );
    shared.write(
        "src/Money.php",
        "<?php\nnamespace Acme\\Lib;\nclass Money {}\n",
    );
    shared.set_clock(1_600_000_000);
    shared.commit("shared");
    let mut app = Fixture::init(&d.path().join("app"));
    app.write("go.mod", "module example.com/app\n");
    app.write(
        "main.go",
        "package main\nimport \"example.com/shared/util\"\n",
    );
    app.write("billing.py", "import acme_lib\n");
    app.write("Pay.php", "<?php\nuse Acme\\Lib\\Money;\n");
    app.write("web.ts", "export {};\n");
    app.set_clock(1_600_000_100);
    app.commit("app");
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: [&app, &shared]
            .iter()
            .map(|f| RepoSpec {
                name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
                path: f.dir.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(s) => assert_eq!(s.steps, 2, "one timeline"),
        SyncOutcome::AlreadyRunning => panic!(),
    }
    let ix = Index::open_read(&w.index_path()).unwrap();
    let links: Vec<String> = ix
        .conn
        .prepare(
            "SELECT l.kind || ' ' || ps.path || ' -> ' || coalesce(pd.path, nd.key)
             FROM interval i JOIN link l ON l.id = i.link
             JOIN node ns ON ns.id = l.src JOIN file_path ps ON ps.file = ns.file
             JOIN node nd ON nd.id = l.dst LEFT JOIN file_path pd ON pd.file = nd.file
             WHERE i.to_seq IS NULL ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        links,
        [
            "import app/Pay.php -> shared/src/Money.php",
            // The distribution's package root holds the module itself: a file link, not a
            // package link to its pyproject.toml.
            "import app/billing.py -> shared/acme_lib/__init__.py",
            "import app/main.go -> module:shared/util",
        ]
    );
    // Read back as of a step, the package end keeps replay's `dir/` name (the remotion gate's form).
    let seq: i64 = ix
        .conn
        .query_row("SELECT max(seq) FROM commits", [], |r| r.get(0))
        .unwrap();
    let at = cartograph_index::build::links_at(&ix.conn, seq as u64).unwrap();
    assert!(
        at.contains(&("import".into(), "app/main.go".into(), "shared/util/".into())),
        "{at:?}"
    );
}

/// The from-scratch oracle on a real history, for a repo in any of the four languages: every
/// `CARTOGRAPH_ORACLE_EVERY`-th commit (default 100) and head.
#[test]
#[ignore = "needs CARTOGRAPH_ORACLE_REPO pointing at a clone"]
fn a_public_repo_replays_equal_to_from_scratch() {
    let path = std::path::PathBuf::from(
        std::env::var("CARTOGRAPH_ORACLE_REPO").expect("CARTOGRAPH_ORACLE_REPO"),
    );
    let every = std::env::var("CARTOGRAPH_ORACLE_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let d = tempfile::tempdir().unwrap();
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let t = std::time::Instant::now();
    let links = replay_equals_from_scratch(&path, &mut cache, every);
    println!("{} links at head; {:?}", links.len(), t.elapsed());
}
