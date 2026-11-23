//! IDX-26 (git hooks) and IDX-27 (the hourly sync's `--all-workspaces` loop), on the binary.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn ok(o: &Output) -> String {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// Registers and indexes `dir` as `cartograph init` does.
fn init(data: &Path, dir: &Path) -> String {
    let plan = cartograph_discovery::setup::plan(dir, None).unwrap();
    let applied = cartograph_discovery::setup::apply(&plan, data).unwrap();
    let ws = cartograph_server::workspace(data, &applied.workspace_id).unwrap();
    let mut cache = FactCache::open(&data.join("cache/facts.db")).unwrap();
    cartograph_index::sync::sync(&ws, &Default::default(), &mut cache).unwrap();
    applied.workspace_id
}

fn repo(root: &Path, name: &str) -> Fixture {
    let mut f = Fixture::init(&root.join(name));
    f.write("a.ts", "import './b';\n");
    f.write("b.ts", "export {};\n");
    f.commit("one");
    f
}

/// `git commit` as a person runs it, so hooks fire; the hook finds the data folder by the same
/// environment.
fn git_commit(f: &Fixture, data: &Path, msg: &str) -> Duration {
    f.git(&["add", "-A"]);
    let t = Instant::now();
    let s = Command::new("git")
        .arg("-C")
        .arg(&f.dir)
        .args(["-c", "commit.gpgsign=false", "commit", "-q", "-m", msg])
        .env("CARTOGRAPH_HOME", data)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .status()
        .unwrap();
    assert!(s.success());
    t.elapsed()
}

fn indexed_commits(data: &Path, ws: &str) -> i64 {
    let p = cartograph_core::paths::workspace_dir(data, ws).join("index.cartograph");
    let Ok(ix) = cartograph_store::Index::open_read(&p) else {
        return 0;
    };
    ix.conn
        .query_row("SELECT count(*) FROM commits", [], |r| r.get(0))
        .unwrap_or(0)
}

#[cfg(unix)]
#[test]
fn a_commit_starts_one_background_sync_and_remove_restores_the_hooks() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let mut f = repo(&root, "app");
    // A hook of the user's own, without a final newline.
    let own = "#!/bin/sh\necho mine >/dev/null";
    std::fs::write(f.dir.join(".git/hooks/post-merge"), own).unwrap();
    let ws = init(&data, &f.dir);
    let dir = f.dir.to_str().unwrap();
    let text = ok(&cartograph(&data, &["hooks", "install", dir]));
    assert!(text.contains("app: installed"), "{text}");
    ok(&cartograph(&data, &["hooks", "install", dir]));
    let merge = std::fs::read_to_string(f.dir.join(".git/hooks/post-merge")).unwrap();
    assert_eq!(
        merge.matches("# cartograph:start").count(),
        1,
        "installing twice is one block"
    );
    assert!(ok(&cartograph(&data, &["hooks", "status", dir])).contains("app: installed"));

    let before = indexed_commits(&data, &ws);
    f.write("c.ts", "export {};\n");
    let took = git_commit(&f, &data, "two");
    let t = Instant::now();
    while indexed_commits(&data, &ws) == before {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the hook's sync never ran"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("perf,hook_commit_overhead,{},30,ms", took.as_millis());
    // The hook's overhead is the D2 lock try and a spawn: git commit doesn't wait for the sync.
    if !cfg!(debug_assertions) {
        assert!(
            took < Duration::from_millis(500),
            "git commit took {took:?}"
        );
    }

    ok(&cartograph(&data, &["hooks", "remove", dir]));
    assert_eq!(
        std::fs::read_to_string(f.dir.join(".git/hooks/post-merge")).unwrap(),
        own
    );
    assert!(
        !f.dir.join(".git/hooks/post-commit").exists(),
        "a hook we made is gone again"
    );
    assert!(ok(&cartograph(&data, &["hooks", "status", dir])).contains("app: absent"));
    let _ = f.commit("keeps the fixture's clock moving");
}

#[test]
fn a_husky_repo_is_left_untouched() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let f = repo(&root, "web");
    std::fs::create_dir_all(f.dir.join(".husky")).unwrap();
    std::fs::write(f.dir.join(".husky/pre-commit"), "npm test\n").unwrap();
    init(&data, &f.dir);
    let hooks: Vec<_> = std::fs::read_dir(f.dir.join(".git/hooks"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let text = ok(&cartograph(
        &data,
        &["hooks", "install", f.dir.to_str().unwrap()],
    ));
    assert!(text.contains("husky owns the hooks"), "{text}");
    assert!(
        text.contains("sync --quiet --background"),
        "prints the snippet: {text}"
    );
    let after: Vec<_> = std::fs::read_dir(f.dir.join(".git/hooks"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(after, hooks);
    assert_eq!(
        std::fs::read_to_string(f.dir.join(".husky/pre-commit")).unwrap(),
        "npm test\n"
    );
}

/// A rebase in progress and a checkout of files start nothing until the rebase ends.
#[test]
fn hooks_during_a_rebase_start_no_sync() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let f = repo(&root, "app");
    let ws = init(&data, &f.dir);
    let ws_dir = cartograph_core::paths::workspace_dir(&data, &ws);
    let dirty = ws_dir.join("dirty");
    let _ = std::fs::remove_file(&dirty);
    let hook = |args: &str| -> serde_json::Value {
        let o = cartograph(
            &data,
            &[
                "sync",
                "--quiet",
                "--background",
                "--json",
                &format!("--hook-args={args}"),
                f.dir.to_str().unwrap(),
            ],
        );
        serde_json::from_str(&ok(&o)).unwrap()
    };
    std::fs::create_dir_all(f.dir.join(".git/rebase-merge")).unwrap();
    for _ in 0..20 {
        assert_eq!(hook("")["background"], "skipped");
    }
    assert!(!dirty.exists(), "not even the lock try");
    std::fs::remove_dir_all(f.dir.join(".git/rebase-merge")).unwrap();
    assert_eq!(
        hook("aaa bbb 0")["background"],
        "skipped",
        "a file checkout"
    );
    assert_ne!(
        hook("rebase")["background"],
        "skipped",
        "post-rewrite after the rebase"
    );
}

/// IDX-27: the hourly job's loop goes on past a repo whose remote can't be reached.
#[test]
fn all_workspaces_fetch_goes_on_past_an_unreachable_remote() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let a = repo(&root.join("one"), "a");
    a.git(&[
        "remote",
        "add",
        "origin",
        root.join("nowhere.git").to_str().unwrap(),
    ]);
    let b = repo(&root.join("two"), "b");
    init(&data, &a.dir);
    init(&data, &b.dir);
    let o = cartograph(&data, &["sync", "--fetch", "--all-workspaces", "--json"]);
    let j: serde_json::Value = serde_json::from_str(&ok(&o)).unwrap();
    let rows = j["workspaces"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{j}");
    assert!(rows.iter().all(|r| r["outcome"] == "synced"), "{j}");
    assert!(
        rows.iter()
            .any(|r| r["detail"].as_str().unwrap().contains("fetch failed: a")),
        "{j}"
    );
}
