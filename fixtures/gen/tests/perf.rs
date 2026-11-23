//! HD-09 performance gates at the library level: the replay of new commits on an indexed history.
//!
//! Method (noise-tolerant on shared runners):
//! - counts gate everywhere, exactly: the resolutions the replay does per new commit must not grow
//!   with the tree, which is what an O(tree) step in replay looks like;
//! - wall clock gates only in an optimised build (`cargo nextest run --cargo-profile perf`, CI's
//!   "Performance gates" job): the best of several interleaved runs, so one slow moment on a shared
//!   runner doesn't fail a pull request. A debug build skips them.
//!
//! Each measurement is printed as `perf,<gate>,<value>,<budget>,<unit>` and, when
//! `CARTOGRAPH_PERF_CSV` names a file, appended to it (CI uploads it; a release appends it to
//! `bench/perf.csv`).

mod support;

use std::path::Path;
use std::time::Instant;

use cartograph_facts::{FactCache, Replay};
use cartograph_git::Repo;
use cartograph_index::build::{changes_of, walk_and_extract};
use support::{import, wide_history};

/// New commits per measurement: the research's "100 new commits on an indexed history".
const NEW: usize = 100;
const SMALL: usize = 200;
const LARGE: usize = 3200;

pub fn record(gate: &str, value: f64, budget: f64, unit: &str) {
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

/// A repository of `files` files and `NEW` later commits, its steps' changes ready to replay.
struct Prepared {
    changes: Vec<Vec<cartograph_facts::replay::Change>>,
}

fn prepare(root: &Path, files: usize) -> Prepared {
    let dir = root.join(format!("wide-{files}"));
    import(&dir, &wide_history(files, NEW));
    let repo = Repo::open(&dir).unwrap();
    let mut cache = FactCache::open(&root.join(format!("facts-{files}.db"))).unwrap();
    let (steps, facts, ..) = walk_and_extract(&repo, repo.head().unwrap(), &mut cache, 4).unwrap();
    assert_eq!(steps.len(), NEW + 1);
    Prepared {
        changes: steps.iter().map(|s| changes_of(s, &facts)).collect(),
    }
}

/// Replays the first commit (the whole tree), then times the `NEW` commits after it. Returns the
/// milliseconds and the resolutions those commits did.
fn replay_new(p: &Prepared) -> (f64, u64) {
    let mut r = Replay::default();
    r.apply(&p.changes[0], 0);
    let before = r.stats.resolutions;
    let t = Instant::now();
    for (seq, c) in p.changes.iter().enumerate().skip(1) {
        r.apply(c, seq as u64);
    }
    (
        t.elapsed().as_secs_f64() * 1e3,
        r.stats.resolutions - before,
    )
}

#[test]
fn perf_replay_of_new_commits_does_not_grow_with_the_tree() {
    let d = tempfile::tempdir().unwrap();
    let small = prepare(d.path(), SMALL);
    let large = prepare(d.path(), LARGE);

    // Counts: each new commit rewrites one file with two imports, so a replay that only touches
    // what changed resolves about the same number of references on either tree. 3% slack.
    let (_, rs) = replay_new(&small);
    let (_, rl) = replay_new(&large);
    record(
        "replay_resolutions_per_commit",
        rl as f64 / NEW as f64,
        0.0,
        "count",
    );
    assert!(
        rl as f64 <= rs as f64 * 1.03 + 2.0,
        "the replay resolved {rl} references for {NEW} commits on a {LARGE}-file tree and {rs} on a \
         {SMALL}-file tree: something re-resolves the whole tree per commit"
    );

    if cfg!(debug_assertions) {
        eprintln!("wall-clock gates skipped in a debug build; CI's perf job runs them");
        return;
    }
    // Wall clock: best of 7, small and large interleaved so drift hits both alike.
    let (mut ts, mut tl) = (f64::MAX, f64::MAX);
    for _ in 0..7 {
        ts = ts.min(replay_new(&small).0);
        tl = tl.min(replay_new(&large).0);
    }
    record("replay_100_commits_ms", tl, 105.0, "ms");
    // ponytail: 16x the tree may cost at most 2x the time (hash maps grow, caches miss), plus half a
    // millisecond of timer noise; an O(tree) step costs about 16x. The research's budget for 100
    // commits is 105 ms.
    assert!(
        tl <= ts * 2.0 + 0.5,
        "100 commits took {tl:.1} ms on {LARGE} files and {ts:.1} ms on {SMALL}: time grows with the tree"
    );
    assert!(
        tl <= 105.0,
        "100 new commits took {tl:.1} ms; the budget is 105 ms"
    );
}
