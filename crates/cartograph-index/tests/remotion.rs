//! The research's six checks in Rust (IDX-06) and the stage 1 gate (IDX-07), on remotion. Run with
//! `CARTOGRAPH_REMOTION=<clone> cargo test --release -p cartograph-index --test remotion -- --ignored --nocapture`.
//! Set `CARTOGRAPH_ABLATION=1` to also replay without the reverse index (slow).
//! Everything is written under a temporary CARTOGRAPH_HOME, never the real ~/.cartograph.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use cartograph_facts::{FactCache, Link, Replay, from_scratch};
use cartograph_git::Repo;
use cartograph_index::build::{all_intervals, changes_of, links_at, seq_of, walk_and_extract};
use cartograph_index::{build_single, default_workers};
use cartograph_parse::{extractor_for, tracked};
use cartograph_store::Index;

fn as_rows(links: &BTreeSet<Link>) -> Vec<(String, String, String)> {
    let mut v: Vec<_> = links
        .iter()
        .map(|(k, a, b)| (k.as_str().to_owned(), a.clone(), b.clone()))
        .collect();
    v.sort();
    v
}

#[test]
#[ignore = "needs CARTOGRAPH_REMOTION pointing at a remotion clone"]
fn remotion_six_checks_and_the_stage_1_gate() {
    let path = std::env::var("CARTOGRAPH_REMOTION").expect("CARTOGRAPH_REMOTION");
    let home = tempfile::tempdir().unwrap();
    let cache_path = cartograph_core::paths::facts_cache(home.path());
    let repo = Repo::open(path.as_ref()).unwrap();
    let to = repo.resolve("2b1b0c9").unwrap();

    // The stage 1 gate: one index file, cold then warm cache.
    let out = home.path().join("workspaces/remotion/index.cartograph");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut cache = FactCache::open(&cache_path).unwrap();
    let cold = build_single(
        path.as_ref(),
        Some("2b1b0c9"),
        &out,
        &mut cache,
        default_workers(),
    )
    .unwrap();
    println!("cold: {cold:?}");
    let warm = build_single(
        path.as_ref(),
        Some("2b1b0c9"),
        &home.path().join("warm.cartograph"),
        &mut cache,
        default_workers(),
    )
    .unwrap();
    println!(
        "warm: total {} ms (walk {}, extract {}, replay and write {}), parsed {}",
        warm.total_ms, warm.walk_ms, warm.extract_ms, warm.replay_ms, warm.extract.parsed
    );
    assert_eq!(warm.extract.parsed, 0);
    println!(
        "index file: {} bytes",
        std::fs::metadata(&out).unwrap().len()
    );

    // The six checks, on an instrumented replay.
    let (steps, facts, _, _, _) =
        walk_and_extract(&repo, to, &mut cache, default_workers()).unwrap();
    let n = steps.len();
    assert_eq!(n, 9_498);
    let samples: Vec<usize> = (1..=24).map(|i| i * n / 24 - 1).collect();
    let mut replay = Replay::default();
    let mut closed = Vec::new();
    let mut snaps: HashMap<usize, BTreeSet<Link>> = HashMap::new();
    let t = Instant::now();
    for (seq, step) in steps.iter().enumerate() {
        replay.apply(&changes_of(step, &facts), seq as u64);
        closed.append(&mut replay.closed);
        if samples.contains(&seq) {
            snaps.insert(seq, replay.links());
        }
    }
    let replay_ms = t.elapsed().as_millis();
    let intervals = all_intervals(&replay, &closed, n as u64);
    println!(
        "replay {replay_ms} ms; resolutions {}; most in one step {}; intervals {} ({} open at head); live refs at head (links, external, builtin, unresolved) {:?}",
        replay.stats.resolutions,
        replay.stats.max_in_step,
        intervals.len(),
        replay.open().count(),
        replay.outcomes()
    );
    let ix = Index::open_read(&out).unwrap();
    let mut from_scratch_ms = 0;
    for &s in &samples {
        let sha = steps[s].sha;
        // 1. The tree equals git's at the sampled commit (paths and blobs).
        let tree = repo.tree_at(sha, tracked).unwrap();
        // 2. Replayed links equal a from-scratch index of that commit, read straight from git.
        let direct: HashMap<String, Arc<_>> = tree
            .iter()
            .map(|(p, o)| {
                (
                    p.clone(),
                    Arc::clone(&facts[&(*o, extractor_for(p).unwrap().1)]),
                )
            })
            .collect();
        let t = Instant::now();
        let want = from_scratch(&direct);
        from_scratch_ms += t.elapsed().as_millis();
        assert_eq!(snaps[&s], want, "links at step {s}");
        // 3. The written intervals reproduce the link set at that step.
        let got: BTreeSet<Link> = intervals
            .iter()
            .filter(|i| i.from <= s as u64 && (s as u64) < i.to)
            .map(|i| i.link.clone())
            .collect();
        assert_eq!(got, want, "intervals at step {s}");
        // The gate: links as of <sha>, queried from the file, equal the from-scratch index.
        let seq = seq_of(&ix.conn, sha).unwrap().unwrap();
        assert_eq!(
            links_at(&ix.conn, seq).unwrap(),
            as_rows(&want),
            "index as of {sha}"
        );
    }
    let head_tree = repo.tree_at(to, tracked).unwrap();
    let replayed: BTreeSet<&String> = replay.tree().keys().collect();
    assert_eq!(replayed, head_tree.keys().collect(), "tree at head");
    println!(
        "checks 1-3 and the as-of query: equal at {} sampled commits; from scratch takes {} ms per commit",
        samples.len(),
        from_scratch_ms / samples.len() as u128
    );

    // 5. An incremental run over the last 100 commits ends in the same state.
    let k = 100;
    let mut base = Replay::default();
    let mut base_closed = Vec::new();
    for (seq, step) in steps[..n - k].iter().enumerate() {
        base.apply(&changes_of(step, &facts), seq as u64);
        base_closed.append(&mut base.closed);
    }
    let t = Instant::now();
    let fresh: Vec<_> = repo
        .walk_first_parent(Some(steps[n - k - 1].sha), to, tracked)
        .unwrap()
        .with_prev_corrected(steps[n - k - 1].corrected_time)
        .map(Result::unwrap)
        .collect();
    assert_eq!(fresh, steps[n - k..]);
    for (i, step) in fresh.iter().enumerate() {
        base.apply(&changes_of(step, &facts), (n - k + i) as u64);
        base_closed.append(&mut base.closed);
    }
    println!(
        "incremental: last {k} commits in {} ms",
        t.elapsed().as_millis()
    );
    assert_eq!(all_intervals(&base, &base_closed, n as u64), intervals);

    // 4. The replay without the reverse index writes identical intervals, over the first
    // CARTOGRAPH_ABLATION steps (default 400): the full ablation would take about an hour.
    let m: usize = std::env::var("CARTOGRAPH_ABLATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400)
        .min(n);
    let (mut rev, mut ab) = (Replay::default(), Replay::new(false));
    let (mut rev_closed, mut ab_closed) = (Vec::new(), Vec::new());
    let t = Instant::now();
    for (seq, step) in steps[..m].iter().enumerate() {
        let c = changes_of(step, &facts);
        rev.apply(&c, seq as u64);
        rev_closed.append(&mut rev.closed);
        ab.apply(&c, seq as u64);
        ab_closed.append(&mut ab.closed);
    }
    println!(
        "ablation over the first {m} steps: {} resolutions without the reverse index, {} with it; identical intervals ({} ms)",
        ab.stats.resolutions,
        rev.stats.resolutions,
        t.elapsed().as_millis()
    );
    assert_eq!(
        all_intervals(&ab, &ab_closed, m as u64),
        all_intervals(&rev, &rev_closed, m as u64)
    );
}
