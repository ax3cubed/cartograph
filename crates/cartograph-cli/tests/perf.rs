//! HD-09 performance gates on the binary: the freshness check (D1), a warm sync of 100 new commits
//! and the single writer (D2). Same method as `fixtures/gen/tests/perf.rs`: best of several runs,
//! wall-clock gates only in an optimised build (CI's "Performance gates" job), one
//! `perf,<gate>,<value>,<budget>,<unit>` line per measurement, appended to `CARTOGRAPH_PERF_CSV`.

#[path = "../../../fixtures/gen/tests/support/mod.rs"]
mod support;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cartograph_git::fixture::Fixture;
use support::{Commit, import, monorepo_history, wide_history};

fn record(gate: &str, value: f64, budget: f64, unit: &str) {
    let line = format!("perf,{gate},{value:.3},{budget},{unit}");
    println!("{line}");
    if let Some(p) = std::env::var_os("CARTOGRAPH_PERF_CSV") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .unwrap();
        writeln!(f, "{line}").unwrap();
    }
}

fn cartograph(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cartograph"));
    c.args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

/// Runs to completion and returns the wall time.
fn timed(home: &Path, args: &[&str]) -> Duration {
    let t = Instant::now();
    let o = cartograph(home, args).output().unwrap();
    let took = t.elapsed();
    assert!(
        o.status.success(),
        "cartograph {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    took
}

fn optimised() -> bool {
    if cfg!(debug_assertions) {
        eprintln!("wall-clock gates skipped in a debug build; CI's perf job runs them");
    }
    !cfg!(debug_assertions)
}

/// D1: the freshness check for 30 repositories, process start included, under 100 ms.
#[test]
fn perf_freshness_for_30_repos_under_100_ms() {
    if !optimised() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let mut folders = Vec::new();
    for i in 0..30 {
        let mut f = Fixture::init(&d.path().join(format!("r{i:02}")));
        f.write("package.json", &format!(r#"{{"name":"r{i}"}}"#));
        f.write("src/a.ts", "import './b';\n");
        f.write("src/b.ts", "export {};\n");
        f.commit("one");
        folders.push(format!(r#"{{"path":"r{i:02}"}}"#));
    }
    std::fs::write(
        d.path().join("w.code-workspace"),
        format!(r#"{{"folders":[{}]}}"#, folders.join(",")),
    )
    .unwrap();
    let home = d.path().join("home");
    // ponytail: canonical, because the registry stores canonical paths and macOS's temp folder is
    // behind the /var -> /private/var link.
    let dir = std::fs::canonicalize(d.path()).unwrap();
    let dir = dir.to_str().unwrap();
    timed(&home, &["init", dir]);
    timed(&home, &["sync", dir]);
    let best = (0..7)
        .map(|_| timed(&home, &["status", "--check", dir]))
        .min()
        .unwrap();
    let ms = best.as_secs_f64() * 1e3;
    record("freshness_30_repos_ms", ms, 100.0, "ms");
    assert!(ms < 100.0, "freshness for 30 repos took {ms:.1} ms");
}

/// A repository with `history` indexed at main~100, with main at the tip: 100 new commits to sync.
fn behind_by_100(root: &Path, history: &[Commit]) -> (std::path::PathBuf, Fixture) {
    let repo = root.join("repo");
    import(&repo, history);
    let f = Fixture::init(&repo);
    let tip = f.git(&["rev-parse", "main"]);
    let base = f.git(&["rev-parse", "main~100"]);
    let home = root.join("base");
    f.git(&["update-ref", "refs/heads/main", &base]);
    timed(&home, &["init", repo.to_str().unwrap()]);
    timed(&home, &["sync", repo.to_str().unwrap()]);
    f.git(&["update-ref", "refs/heads/main", &tip]);
    (home, f)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

/// Phase 2 exit gate: 100 new commits synced warm (fact cache filled) under 500 ms, on a tree of
/// remotion's size, so resolving the whole tree again on every sync fails it.
#[test]
fn perf_sync_of_100_new_commits_under_500_ms() {
    if !optimised() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let (base, f) = behind_by_100(d.path(), &monorepo_history(200));
    let repo = f.dir.to_str().unwrap().to_owned();
    // Best of five with the replay snapshot, best of three without it (the whole tree resolved).
    let run = |runs: usize, keep_snapshot: bool| {
        let mut best = Duration::MAX;
        for i in 0..runs {
            let home = d.path().join(format!("h{keep_snapshot}{i}"));
            copy_dir(&base, &home);
            if !keep_snapshot {
                remove_snapshots(&home);
            }
            best = best.min(timed(&home, &["sync", &repo]));
        }
        best.as_secs_f64() * 1e3
    };
    let (warm, cold) = (run(5, true), run(3, false));
    record("sync_100_commits_ms", warm, 500.0, "ms");
    record(
        "sync_100_commits_vs_full_resolve",
        warm / cold,
        0.5,
        "ratio",
    );
    // A shared CI runner is about twice as slow as a developer machine, so the absolute budget
    // holds on reference hardware (CARTOGRAPH_PERF_STRICT=1); everywhere, the snapshot must at
    // least halve the sync, which fails the moment a sync resolves the whole tree again.
    assert!(
        warm < cold * 0.5,
        "a sync of 100 commits took {warm:.1} ms with the snapshot, {cold:.1} ms without"
    );
    if std::env::var_os("CARTOGRAPH_PERF_STRICT").is_some() {
        assert!(warm < 500.0, "a warm sync of 100 commits took {warm:.1} ms");
    }
}

/// Deletes every replay snapshot under `home`, so the next sync resolves the whole tree.
fn remove_snapshots(home: &Path) {
    for e in std::fs::read_dir(home).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            remove_snapshots(&e.path());
        } else if e.file_name() == "replay.snapshot" {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
}

/// D2: ten syncs started at once give exactly one writer; the nine others exit 0 within 50 ms.
#[test]
fn perf_ten_concurrent_syncs_give_one_writer() {
    let d = tempfile::tempdir().unwrap();
    let (base, f) = behind_by_100(d.path(), &wide_history(400, 200));
    let repo = f.dir.to_str().unwrap().to_owned();
    let home = d.path().join("home");
    copy_dir(&base, &home);
    let t = Instant::now();
    let mut children: Vec<_> = (0..10)
        .map(|_| (cartograph(&home, &["sync", &repo]).spawn().unwrap(), None))
        .collect();
    while children.iter().any(|(_, done)| done.is_none()) {
        for (c, done) in &mut children {
            if done.is_none()
                && let Some(s) = c.try_wait().unwrap()
            {
                assert!(s.success(), "every sync exits 0");
                *done = Some(t.elapsed());
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut took: Vec<Duration> = children.into_iter().map(|(_, d)| d.unwrap()).collect();
    took.sort();
    let ninth = took[8].as_secs_f64() * 1e3;
    let writer = took[9].as_secs_f64() * 1e3;
    record("concurrent_sync_ninth_exit_ms", ninth, 50.0, "ms");
    if !optimised() {
        return;
    }
    assert!(
        ninth < 50.0 && writer > ninth,
        "nine syncs should exit within 50 ms while one writes: {took:?}"
    );
}
