//! HD-06 properties that need the binary: SIGKILL at random points, and incremental equals full.
//! Histories are generated (`fixtures/gen/tests/support`); prefixes of one history are exposed by
//! moving `main`, so a split never rewrites a commit.
//!
//! Case counts follow `PROPTEST_CASES` (256 by default, 10000 nightly); kill points follow
//! `CARTOGRAPH_KILL_POINTS` (100, the phase 2 exit gate). Failing seeds are saved to
//! `crates/cartograph-cli/tests/props.proptest-regressions`.

#[path = "../../../fixtures/gen/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cartograph_fixtures::fact_hash;
use cartograph_git::fixture::Fixture;
use cartograph_store::rusqlite::{Connection, OpenFlags};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use support::{Commit, import, wide_history};

fn cartograph(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cartograph"));
    c.args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

fn run_ok(home: &Path, args: &[&str]) {
    let o = cartograph(home, args).output().unwrap();
    assert!(
        o.status.success(),
        "cartograph {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// `cartograph init`, then the first sync, which builds the index.
fn init_and_sync(home: &Path, repo: &str) {
    run_ok(home, &["init", repo]);
    run_ok(home, &["sync", repo]);
}

/// The one index under `home`.
fn the_index(home: &Path) -> PathBuf {
    let found: Vec<PathBuf> = std::fs::read_dir(home.join("workspaces"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("index.cartograph"))
        .filter(|p| p.is_file())
        .collect();
    assert_eq!(found.len(), 1, "one index in {}", home.display());
    found.into_iter().next().unwrap()
}

fn hash_of(index: &Path) -> String {
    fact_hash(index)
}

/// `PRAGMA quick_check` on a file that may be half written; also checks the WAL beside it.
fn quick_check(index: &Path) -> String {
    let c = Connection::open_with_flags(index, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
        .unwrap_or_else(|e| e.to_string())
}

fn kill_points() -> usize {
    std::env::var("CARTOGRAPH_KILL_POINTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

/// A tiny seeded generator, so a failing kill point can be replayed from the printed seed.
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// The SIGKILL harness: runs `args` to completion once to learn its duration, then `points` times
/// runs `before(home)`, starts `args` and kills it (SIGKILL on Unix, TerminateProcess on Windows)
/// at a random moment inside that duration. After each kill, every index under `home` must pass
/// `quick_check`, and `after(home)` (the next sync, or nothing) must succeed. Returns how many runs
/// were really cut short.
fn kill_at_random_points(
    home: &Path,
    args: &[&str],
    before: &dyn Fn(&Path),
    after: &dyn Fn(&Path),
) -> usize {
    before(home);
    let t = Instant::now();
    run_ok(home, args);
    let full = t.elapsed();
    let mut seed = std::env::var("CARTOGRAPH_KILL_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x9e37_79b9_7f4a_7c15u64);
    eprintln!("kill seed {seed} (CARTOGRAPH_KILL_SEED), full run {full:?}");
    let mut cut = 0;
    for point in 0..kill_points() {
        let at = Duration::from_micros(next(&mut seed) % full.as_micros().max(1) as u64);
        before(home);
        let mut child = cartograph(home, args)
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(at);
        if child.try_wait().unwrap().is_none() {
            cut += 1;
        }
        let _ = child.kill();
        let _ = child.wait();
        for e in std::fs::read_dir(home.join("workspaces"))
            .unwrap()
            .flatten()
        {
            let ix = e.path().join("index.cartograph");
            if ix.is_file() {
                assert_eq!(quick_check(&ix), "ok", "point {point}, killed after {at:?}");
            }
        }
        after(home);
    }
    cut
}

/// The rebuild path writes a new file and renames it into place (B2), so a kill at any moment
/// leaves the old index intact and readable, with the same facts.
#[test]
fn sigkill_during_a_rebuild_leaves_the_old_index_intact() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    import(&repo, &wide_history(150, 60));
    let home = d.path().join("home");
    let repo_s = repo.to_str().unwrap();
    run_ok(&home, &["index", repo_s]);
    let ix = the_index(&home);
    let want = hash_of(&ix);
    let cut = kill_at_random_points(
        &home,
        &["index", "--rebuild", "--yes", repo_s],
        &|_| {},
        &|_| {},
    );
    assert_eq!(hash_of(&the_index(&home)), want);
    assert!(
        cut > 0,
        "no run was cut short; the kill points are too late"
    );
}

/// Exit gate (phase 2): SIGKILL at 100 random points of a sync never leaves an index that fails
/// quick_check, and the next sync ends with the fact hash of an uninterrupted run.
#[test]
fn sigkill_during_sync_then_sync_again_gives_the_uninterrupted_hash() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    let commits = wide_history(150, 120);
    import(&repo, &commits);
    let f = Fixture::init(&repo);
    let tip = f.git(&["rev-parse", "main"]);
    let base = f.git(&["rev-parse", "main~100"]);
    let repo_s = repo.to_str().unwrap();

    let clean = d.path().join("clean");
    init_and_sync(&clean, repo_s);
    let want = hash_of(&the_index(&clean));

    // `base` holds the index of main~100; each kill interrupts a sync of the same 100 new commits
    // into a fresh copy of it.
    let base_home = d.path().join("base");
    f.git(&["update-ref", "refs/heads/main", &base]);
    init_and_sync(&base_home, repo_s);
    f.git(&["update-ref", "refs/heads/main", &tip]);
    let home = d.path().join("home");
    let restore = |home: &Path| {
        let _ = std::fs::remove_dir_all(home);
        copy_dir(&base_home, home);
    };
    let cut = kill_at_random_points(&home, &["sync", repo_s], &restore, &|home| {
        run_ok(home, &["sync", repo_s]);
        assert_eq!(hash_of(&the_index(home)), want, "the sync after a kill");
    });
    assert!(cut > 0, "no sync was cut short");
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

fn split_config() -> ProptestConfig {
    // ponytail: on macOS `init` waits about 10 s on `tmutil addexclusion`
    // (cartograph-core::paths::exclude_from_backup), so 256 cases would take over an hour there;
    // 16 until that call stops blocking. PRs run on Linux with the full 256.
    let mac = cfg!(target_os = "macos") && std::env::var_os("PROPTEST_CASES").is_none();
    ProptestConfig {
        cases: if mac {
            16
        } else {
            ProptestConfig::default().cases
        },
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/props.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

/// A small random history: each commit rewrites one of six files with some of the others'
/// imports, or deletes it.
fn random_history(ops: &[(usize, u8)]) -> Vec<Commit> {
    let mut out = vec![Commit::from([(
        "package.json".to_owned(),
        Some(r#"{"name":"p","private":true}"#.to_owned()),
    )])];
    for &(f, mask) in ops {
        let body = (mask != 0).then(|| {
            (0..6)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| format!("import x{i} from './f{i}';\n"))
                .collect::<String>()
        });
        out.push(Commit::from([(format!("src/f{f}.ts"), body)]));
    }
    out
}

proptest! {
    #![proptest_config(split_config())]

    /// Exit gate (phase 2): any split of a history into sync runs gives one fact hash.
    #[test]
    fn incremental_equals_full_for_any_split(
        ops in prop::collection::vec((0..6usize, 0..64u8), 2..16),
        cuts in prop::collection::btree_set(1..16usize, 0..4),
    ) {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        import(&repo, &random_history(&ops));
        let f = Fixture::init(&repo);
        let shas: Vec<String> = f.git(&["rev-list", "--reverse", "main"]).lines().map(str::to_owned).collect();
        let repo_s = repo.to_str().unwrap();

        let full = d.path().join("full");
        init_and_sync(&full, repo_s);
        let want = hash_of(&the_index(&full));

        let split = d.path().join("split");
        run_ok(&split, &["init", repo_s]);
        for &c in cuts.iter().filter(|&&c| c < shas.len()).chain([&(shas.len() - 1)]) {
            f.git(&["update-ref", "refs/heads/main", &shas[c]]);
            run_ok(&split, &["sync", repo_s]);
        }
        prop_assert_eq!(hash_of(&the_index(&split)), want);
    }
}
