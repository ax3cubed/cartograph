//! The IDX-14 gate at remotion's size without a remotion clone: 100 new commits on a generated
//! 9,000-file monorepo, synced warm, in-process (the binary's gate is `perf_sync_of_100_new_commits`
//! in cartograph-cli). Run in an optimised build:
//! `cargo test --profile perf -p cartograph-index --test sync_scale -- --ignored --nocapture`.

#[path = "../../../fixtures/gen/tests/support/mod.rs"]
mod support;

use std::time::Instant;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use support::{import, monorepo_history};

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

#[test]
#[ignore = "scale gate: run in an optimised build with --ignored"]
fn a_hundred_new_commits_warm_at_remotion_scale() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    import(&repo, &monorepo_history(300));
    let f = Fixture::init(&repo);
    let tip = f.git(&["rev-parse", "main"]);
    f.git(&[
        "update-ref",
        "refs/heads/main",
        &f.git(&["rev-parse", "main~100"]),
    ]);
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: repo.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let first = run(&w, &mut cache);
    println!("first sync: {} steps in {} ms", first.steps, first.total_ms);
    f.git(&["update-ref", "refs/heads/main", &tip]);
    let t = Instant::now();
    let s = run(&w, &mut cache);
    let ms = t.elapsed().as_millis();
    println!(
        "100 new commits at remotion scale: {ms} ms (rehydrate {}, walk {}, extract+replay+write {})",
        s.rehydrate_ms, s.walk_ms, s.replay_ms
    );
    assert_eq!(s.steps, 100);
    assert!(
        s.restored,
        "the replay is restored from the last sync's snapshot"
    );
    if !cfg!(debug_assertions) {
        assert!(ms < 500, "a warm sync of 100 commits took {ms} ms");
    }
}

/// The snapshot is used only when it matches the index; anything else falls back to resolving the
/// tree, and every path gives the fact hash of a sync from nothing.
#[test]
fn a_snapshot_is_used_only_when_it_matches_the_index() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("package.json", r#"{"name":"app"}"#);
    f.write("src/a.ts", "import './b';\n");
    f.write("src/b.ts", "export {};\n");
    f.commit("one");
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let hash = |w: &Workspace| {
        let ix = cartograph_store::Index::open_read(&w.index_path()).unwrap();
        cartograph_store::fact_hash(&ix.conn).unwrap().to_string()
    };
    assert!(!run(&w, &mut cache).restored, "nothing to restore from");
    let snap = w.snapshot_path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&snap).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let mut n = 0;
    let mut step = |f: &mut Fixture| {
        n += 1;
        f.write(
            "src/b.ts",
            &format!("import './a';\nexport const n = {n};\n"),
        );
        f.commit("more");
    };
    step(&mut f);
    assert!(run(&w, &mut cache).restored);

    // A damaged body, a header from another index, and no file at all.
    let good = std::fs::read(&snap).unwrap();
    let header = good.iter().position(|b| *b == b'\n').unwrap() + 1;
    let mut damaged = good[..header].to_vec();
    damaged.extend(b"not postcard");
    std::fs::write(&snap, damaged).unwrap();
    step(&mut f);
    assert!(!run(&w, &mut cache).restored, "a damaged body");
    let mut other = std::fs::read(&snap).unwrap();
    let at = other.iter().position(|b| *b == b':').unwrap() + 1;
    other[at] = if other[at] == b'0' { b'1' } else { b'0' };
    std::fs::write(&snap, other).unwrap();
    step(&mut f);
    assert!(!run(&w, &mut cache).restored, "another fact hash");
    std::fs::remove_file(&snap).unwrap();
    step(&mut f);
    assert!(!run(&w, &mut cache).restored, "no snapshot");
    step(&mut f);
    assert!(run(&w, &mut cache).restored);

    let fresh = Workspace {
        dir: d.path().join("fresh"),
        ..w.clone()
    };
    run(&fresh, &mut cache);
    assert_eq!(
        hash(&w),
        hash(&fresh),
        "restored syncs equal a sync from nothing"
    );
}

/// The repo list's `dirty`: a change to a tracked file counts, an untracked file doesn't, and a
/// folder git can't read is unknown.
#[test]
fn uncommitted_changes_to_tracked_files_make_a_repo_dirty() {
    use cartograph_index::freshness::uncommitted;
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "export {};\n");
    f.commit("one");
    assert_eq!(uncommitted(&f.dir), Some(false));
    std::fs::write(f.dir.join("new.ts"), "x").unwrap();
    assert_eq!(uncommitted(&f.dir), Some(false), "untracked");
    std::fs::write(f.dir.join("a.ts"), "export const a = 1;\n").unwrap();
    assert_eq!(uncommitted(&f.dir), Some(true));
    assert_eq!(uncommitted(&d.path().join("nowhere")), None);
}

/// Progress between the start and the end: once per step, counting up to the round's total.
#[test]
fn a_sync_reports_each_step_of_its_round() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    for i in 0..5 {
        f.write("a.ts", &format!("export const a = {i};\n"));
        f.commit("step");
    }
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let mut seen = Vec::new();
    cartograph_index::sync::sync_with_progress(
        &w,
        &SyncOptions::default(),
        &mut cache,
        &mut |done, total| seen.push((done, total)),
    )
    .unwrap();
    assert_eq!(seen, (1..=5).map(|i| (i, 5)).collect::<Vec<_>>());
}
