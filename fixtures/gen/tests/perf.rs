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

/// I1's fixture as an index (`cartograph_fixtures::scale`).
fn scale_index(path: &Path) -> cartograph_store::Index {
    cartograph_fixtures::scale::index(path)
}

/// IDX-25's gate: the 60k-file, 200k-link fixture lays out in under 30 s on a first index and
/// under 1 s after a sync that adds 100 files; positions don't depend on the run.
#[test]
fn perf_layout_of_60k_files_and_200k_links() {
    let d = tempfile::tempdir().unwrap();
    let ix = scale_index(&d.path().join("scale.cartograph"));
    let c = &ix.conn;
    let t = Instant::now();
    let parents = cartograph_index::layout::run(c, 0, None).unwrap();
    let first = t.elapsed().as_secs_f64();
    assert!(parents > 1_000, "every parent was laid out");
    let placed: i64 = c
        .query_row("SELECT count(*) FROM layout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(placed, 62 + 1_140 + 60_000);
    // 100 new files in existing modules: only their parents move anything.
    c.execute_batch("BEGIN").unwrap();
    for f in 60_000..60_100i64 {
        let m = f % 1_140;
        c.execute("INSERT INTO file (id, repo) VALUES (?1, 1)", [f])
            .unwrap();
        c.execute(
            "INSERT INTO node (id, kind, file, parent, service) VALUES (?1, 'file', ?2, ?3, ?4)",
            cartograph_store::rusqlite::params![10_000 + f, f, 1_000 + m, 1 + m % 62],
        )
        .unwrap();
        c.execute(
            "INSERT INTO file_path (file, path, from_seq) VALUES (?1, ?2, 0)",
            cartograph_store::rusqlite::params![f, format!("new/{f}.ts")],
        )
        .unwrap();
    }
    c.execute_batch("COMMIT").unwrap();
    let t = Instant::now();
    let again = cartograph_index::layout::run(c, 1, None).unwrap();
    let sync = t.elapsed().as_secs_f64();
    assert_eq!(
        again, 100,
        "one parent per new file at most, and nothing else"
    );
    let moved: i64 = c
        .query_row(
            "SELECT count(*) FROM layout WHERE to_seq IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(moved, 0, "no existing node moved");
    record("layout_first_index_s", first, 30.0, "s");
    record("layout_100_file_sync_s", sync, 1.0, "s");
    if cfg!(debug_assertions) {
        return;
    }
    assert!(first < 30.0, "a first layout took {first:.1} s");
    assert!(sync < 1.0, "a 100-file sync's layout took {sync:.2} s");
}

/// AI-15's gate: on the 60k-file fixture, "invoice repo" finds InvoiceRepo.ts in the top 3, at a
/// p95 under 20 ms; FTS syntax in a query never errors.
#[test]
fn perf_search_finds_invoice_repo_in_the_top_3() {
    let d = tempfile::tempdir().unwrap();
    let ix = scale_index(&d.path().join("scale.cartograph"));
    let c = &ix.conn;
    c.execute_batch("BEGIN").unwrap();
    let rows: Vec<(i64, String)> = c
        .prepare("SELECT n.id, fp.path FROM node n JOIN file_path fp ON fp.file = n.file")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (id, path) in &rows {
        cartograph_store::search::put(c, *id, path.rsplit('/').next().unwrap(), path).unwrap();
    }
    c.execute_batch("COMMIT").unwrap();
    let mut times = Vec::new();
    for _ in 0..40 {
        let t = Instant::now();
        let hits = cartograph_store::search::search(c, "invoice repo", None, 3).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(hits[0].node, 10_000 + 4_242, "{hits:?}");
    }
    for q in ["\" or NEAR(", "*", "AND OR NOT", "invoice\"", "^"] {
        assert!(
            cartograph_store::search::search(c, q, Some(0), 10).is_ok(),
            "{q}"
        );
    }
    times.sort_by(f64::total_cmp);
    let p95 = times[37];
    record("search_p95_ms", p95, 20.0, "ms");
    if !cfg!(debug_assertions) {
        assert!(p95 < 20.0, "search p95 {p95:.1} ms");
    }
}

/// Phase 3's exit gate: a force-push that rewrites 50 commits ends in the fact hash of a
/// rebuild, and a moved file keeps one lineage. Counts gates: they hold in every build.
#[test]
fn perf_rewrite_of_50_commits_equals_a_rebuild_and_a_move_keeps_its_lineage() {
    use cartograph_git::fixture::Fixture;
    use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    let mut shas = Vec::new();
    for i in 0..100 {
        f.write(
            &format!("src/m{}.ts", i % 10),
            &format!("import './m{}';\nexport const v = {i};\n", (i + 3) % 10),
        );
        shas.push(f.commit(&format!("c{i}")));
    }
    let repo_dir = f.dir.clone();
    let ws = |name: &str| Workspace {
        dir: d.path().join(name),
        repos: vec![cartograph_index::timeline::RepoSpec {
            name: "repo".into(),
            path: repo_dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let run = |w: &Workspace, cache: &mut FactCache| match sync(w, &SyncOptions::default(), cache)
        .unwrap()
    {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!(),
    };
    let inc = ws("inc");
    run(&inc, &mut cache);
    f.git(&["reset", "-q", "--hard", &shas[49]]);
    for i in 0..50 {
        if i == 10 {
            let body = std::fs::read_to_string(f.dir.join("src/m1.ts")).unwrap();
            std::fs::remove_file(f.dir.join("src/m1.ts")).unwrap();
            f.write("src/moved/m1.ts", &body);
            f.write("src/m2.ts", "import './moved/m1';\n");
        } else {
            f.write(
                &format!("src/n{}.ts", i % 7),
                &format!("import './m{}';\nexport const w = {i};\n", i % 10),
            );
        }
        f.commit(&format!("r{i}"));
    }
    let t = Instant::now();
    let s = run(&inc, &mut cache);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(s.rewritten, ["repo"]);
    let fresh = ws("fresh");
    run(&fresh, &mut cache);
    let hash = |w: &Workspace| {
        let ix = cartograph_store::Index::open_read(&w.index_path()).unwrap();
        cartograph_store::fact_hash(&ix.conn).unwrap().to_string()
    };
    let equal = hash(&inc) == hash(&fresh);
    record(
        "rewrite_50_equals_rebuild",
        f64::from(u8::from(equal)),
        1.0,
        "bool",
    );
    record("rewrite_50_sync_ms", ms, 0.0, "ms");
    assert!(equal, "a rewrite of 50 commits differs from a rebuild");
    let ix = cartograph_store::Index::open_read(&inc.index_path()).unwrap();
    let paths: Vec<String> = ix
        .conn
        .prepare(
            "SELECT path FROM file_path WHERE file = (SELECT file FROM file_path
               WHERE path = 'src/moved/m1.ts') ORDER BY from_seq",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let one = paths == ["src/m1.ts", "src/moved/m1.ts"];
    record(
        "moved_file_one_lineage",
        f64::from(u8::from(one)),
        1.0,
        "bool",
    );
    assert!(one, "{paths:?}");
}
