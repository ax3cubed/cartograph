//! IDX-12 (merged timeline, cross-repo package links), IDX-14 (incremental sync) and IDX-15
//! (freshness, dirty flag, background sync), over small fixture repositories made with git.
//!
//! The scale gates are ignored tests, run in release:
//! `cargo test --release -p cartograph-index --test workspace_sync -- --ignored --nocapture`
//! (set `CARTOGRAPH_REMOTION=<clone>` for the remotion one).

use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::time::{Duration, Instant};

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_git::{CommitStep, ObjectId};
use cartograph_index::freshness::{
    self, AutoSync, BEHIND_CAP, State, SyncRequest, request_sync_with,
};
use cartograph_index::sync::{SyncOptions, SyncOutcome, SyncSummary, Workspace, sync};
use cartograph_index::timeline::{RepoSpec, merge};
use cartograph_store::Index;
use cartograph_store::hash::fact_hash;
use cartograph_store::writer::WriterLock;
use proptest::prelude::*;

const T0: i64 = 1_600_000_000;

/// Tests here run one at a time: a git child forked while another test holds a writer lock
/// inherits the lock's file for a moment, which reads as "a sync is running".
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Two repositories with a cross-repo package link, clock skew between them, a committer clock
/// that goes backwards, a tie on corrected time, a deletion and a package that goes away.
/// Returns the fixtures and each one's commits, oldest first.
fn two_repos(root: &Path) -> (Fixture, Vec<String>, Fixture, Vec<String>) {
    let mut api = Fixture::init(&root.join("api"));
    let mut web = Fixture::init(&root.join("web"));
    let (mut a, mut w) = (Vec::new(), Vec::new());
    let at = |f: &mut Fixture, t: i64, msg: &str| {
        f.set_clock(T0 + t - 60);
        f.commit(msg)
    };
    // web is a day behind api's clock all along (skew), so its commits interleave by date.
    api.write("src/index.ts", "export const x = 1;\n");
    api.write("src/b.ts", "export const b = 1;\n");
    a.push(at(&mut api, 1_000, "api: first"));
    web.write("package.json", r#"{"name":"web","private":true}"#);
    web.write("src/app.ts", "import { x } from '@acme/api';\n");
    w.push(at(&mut web, 500, "web: first, the import dangles"));
    api.write(
        "package.json",
        r#"{"name":"@acme/api","exports":"./src/index.ts"}"#,
    );
    a.push(at(&mut api, 2_000, "api: becomes a package"));
    web.write("src/util.ts", "export const u = 1;\n");
    web.write(
        "src/app.ts",
        "import { x } from '@acme/api';\nimport { u } from './util';\n",
    );
    w.push(at(&mut web, 1_500, "web: util"));
    // The committer clock goes backwards: corrected time keeps web's order.
    web.write("src/more.ts", "import { u } from './util';\n");
    w.push(at(&mut web, 1_200, "web: clock went back"));
    api.write(
        "src/index.ts",
        "import { b } from './b';\nexport const x = b;\n",
    );
    a.push(at(&mut api, 3_000, "api: index uses b"));
    // A tie: same corrected time in both repos; "api" sorts first.
    web.write("src/tie.ts", "import { x } from '@acme/api';\n");
    w.push(at(&mut web, 3_500, "web: tie"));
    api.write("src/tie.ts", "export const t = 1;\n");
    a.push(at(&mut api, 3_500, "api: tie"));
    api.remove("package.json");
    a.push(at(&mut api, 5_000, "api: no longer a package"));
    web.remove("src/more.ts");
    web.write("src/app.ts", "import { u } from './util';\n");
    w.push(at(&mut web, 6_000, "web: drops the import"));
    api.write(
        "package.json",
        r#"{"name":"@acme/api","main":"src/index.ts"}"#,
    );
    a.push(at(&mut api, 7_000, "api: a package again"));
    (api, a, web, w)
}

fn ws(dir: &Path, repos: &[&Fixture]) -> Workspace {
    Workspace {
        dir: dir.to_owned(),
        repos: repos
            .iter()
            .map(|f| RepoSpec {
                name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
                path: f.dir.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn opts() -> SyncOptions {
    SyncOptions {
        workers: 2,
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &opts(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

/// The fact hash (IDX-17) and the commit order, which the hash only sees through intervals.
fn fingerprint(index: &Path) -> (String, Vec<String>) {
    let ix = Index::open_read(index).unwrap();
    let order = ix
        .conn
        .prepare("SELECT lower(hex(sha)) FROM commits ORDER BY seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (fact_hash(&ix.conn).unwrap().to_string(), order)
}

/// Points a fixture's branch at its `n`th commit (0: no commits at all).
fn set_tip(f: &Fixture, shas: &[String], n: usize) {
    if n == 0 {
        f.git(&["update-ref", "-d", "refs/heads/main"]);
    } else {
        f.git(&["update-ref", "refs/heads/main", &shas[n - 1]]);
    }
}

fn step(t: i64, n: u8) -> CommitStep {
    CommitStep {
        sha: ObjectId::from_hex(format!("{n:040x}").as_bytes()).unwrap(),
        time: t,
        corrected_time: t,
        author: String::new(),
        summary: String::new(),
        changes: Vec::new(),
        renames: Vec::new(),
        rename_limit: false,
    }
}

proptest! {
    /// IDX-12: the merge is a total order (corrected time, then repo) that keeps each repo's order.
    #[test]
    fn the_merge_is_total_and_keeps_each_repos_order(
        gaps in proptest::collection::vec(proptest::collection::vec(1i64..5, 0..20), 1..5)
    ) {
        let mut n = 0u8;
        let per: Vec<Vec<CommitStep>> = gaps
            .iter()
            .map(|g| {
                let mut t = 0;
                g.iter().map(|d| { t += d; n += 1; step(t, n) }).collect()
            })
            .collect();
        let merged = merge(per.clone());
        prop_assert_eq!(merged.len(), per.iter().map(Vec::len).sum::<usize>());
        for w in merged.windows(2) {
            prop_assert!((w[0].1.corrected_time, w[0].0) < (w[1].1.corrected_time, w[1].0));
        }
        for (r, steps) in per.iter().enumerate() {
            let mine: Vec<_> = merged.iter().filter(|(i, _)| *i == r).map(|(_, s)| s.sha).collect();
            prop_assert_eq!(mine, steps.iter().map(|s| s.sha).collect::<Vec<_>>());
        }
    }
}

fn strings(ix: &Index, sql: &str) -> Vec<String> {
    ix.conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// IDX-12: one timeline across repos; a bare import in web links to api's package while api's
/// package.json declares the name, and every interval opens and closes at a step of the repo whose
/// change caused it.
#[test]
fn a_cross_repo_package_link_opens_and_closes_with_the_package() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let (api, _, web, _) = two_repos(&d.path().join("src"));
    let w = ws(&d.path().join("ws"), &[&web, &api]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let s = run(&w, &mut cache);
    assert_eq!((s.steps, s.watermark), (11, 10));
    let ix = Index::open_read(&w.index_path()).unwrap();
    // By corrected date; web's backwards clock keeps its order; api first on the tie at 3,500.
    assert_eq!(
        strings(&ix, "SELECT summary FROM commits ORDER BY seq"),
        [
            "web: first, the import dangles",
            "api: first",
            "web: util",
            "web: clock went back",
            "api: becomes a package",
            "api: index uses b",
            "api: tie",
            "web: tie",
            "api: no longer a package",
            "web: drops the import",
            "api: a package again",
        ]
    );
    let cross = strings(
        &ix,
        "SELECT ps.path || ' -> ' || pd.path || ' opened by ' || co.summary || ', closed by '
                || coalesce(cc.summary, '-')
         FROM interval i JOIN link l ON l.id = i.link
         JOIN node ns ON ns.id = l.src JOIN file_path ps ON ps.file = ns.file
         JOIN node nd ON nd.id = l.dst JOIN file_path pd ON pd.file = nd.file
         JOIN commits co ON co.seq = i.from_seq LEFT JOIN commits cc ON cc.seq = i.to_seq
         WHERE substr(ps.path, 1, 4) <> substr(pd.path, 1, 4)
         GROUP BY i.rowid ORDER BY i.from_seq, ps.path",
    );
    assert_eq!(
        cross,
        [
            "web/src/app.ts -> api/src/index.ts opened by api: becomes a package, closed by api: no longer a package",
            "web/src/tie.ts -> api/src/index.ts opened by web: tie, closed by api: no longer a package",
            "web/src/tie.ts -> api/src/index.ts opened by api: a package again, closed by -",
        ]
    );
    // Every interval end is a step of the repo that changed a file at that step.
    let stray: i64 = ix
        .conn
        .query_row(
            "SELECT count(*) FROM interval i, commits c
             WHERE c.seq IN (i.from_seq, i.to_seq) AND NOT EXISTS (
               SELECT 1 FROM file_version fv JOIN file f ON f.id = fv.file
                 WHERE f.repo = c.repo AND c.seq IN (fv.from_seq, fv.to_seq)
               UNION ALL SELECT 1 FROM file_path fp JOIN file f ON f.id = fp.file
                 WHERE f.repo = c.repo AND c.seq IN (fp.from_seq, fp.to_seq))",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stray, 0);
    assert!(s.duplicate_packages.is_empty());
}

/// Two repos declaring one package name: the first by repo name wins and the name is reported.
#[test]
fn a_duplicate_package_name_resolves_to_the_first_repo_and_is_reported() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut fork = Fixture::init(&d.path().join("zz-fork"));
    let mut real = Fixture::init(&d.path().join("lib"));
    let mut app = Fixture::init(&d.path().join("app"));
    for f in [&mut real, &mut fork] {
        f.write("package.json", r#"{"name":"lib","main":"index.ts"}"#);
        f.write("index.ts", "export const l = 1;\n");
        // Distinct commits: the schema's unique commits.sha can't hold one commit in two repos yet.
        let name = f.dir.file_name().unwrap().to_string_lossy().into_owned();
        f.commit(&name);
    }
    app.write("main.ts", "import { l } from 'lib';\n");
    app.commit("app");
    let w = ws(&d.path().join("ws"), &[&fork, &app, &real]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let s = run(&w, &mut cache);
    assert_eq!(s.duplicate_packages, ["lib"]);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        cartograph_index::build::links_at(&ix.conn, 2)
            .unwrap()
            .into_iter()
            .map(|(_, a, b)| format!("{a} -> {b}"))
            .collect::<Vec<_>>(),
        ["app/main.ts -> lib/index.ts"]
    );
}

/// IDX-14: any split of a history into sync runs, including runs that see a repo late or move a
/// branch backwards, ends with the fact hash and commit order of one full sync.
#[test]
fn any_split_into_sync_runs_gives_the_full_fact_hash() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let (api, a, web, w) = two_repos(&d.path().join("src"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let full_ws = ws(&d.path().join("full"), &[&api, &web]);
    run(&full_ws, &mut cache);
    let full = fingerprint(&full_ws.index_path());
    let case = std::cell::Cell::new(0);
    let cache = std::cell::RefCell::new(cache);
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig {
        cases: 48,
        ..ProptestConfig::default()
    });
    let splits = proptest::collection::vec((0..=a.len(), 0..=w.len()), 1..4);
    runner
        .run(&splits, |runs| {
            case.set(case.get() + 1);
            let split_ws = ws(
                &d.path().join(format!("split-{}", case.get())),
                &[&api, &web],
            );
            for (na, nw) in runs.iter().copied().chain([(a.len(), w.len())]) {
                set_tip(&api, &a, na);
                set_tip(&web, &w, nw);
                run(&split_ws, &mut cache.borrow_mut());
            }
            prop_assert_eq!(&fingerprint(&split_ws.index_path()), &full);
            Ok(())
        })
        .unwrap();
}

/// IDX-14: 100 new commits on top of an indexed history give the full build's hash, and a
/// force-push replays from the merge base.
#[test]
fn a_hundred_new_commits_and_a_force_push_match_a_full_sync() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    let mut shas = Vec::new();
    for i in 0..120 {
        let n = i % 7;
        f.write(
            &format!("src/m{n}.ts"),
            &format!(
                "import {{ a }} from './m{}';\nexport const v{i} = 1;\n",
                (n + 1) % 7
            ),
        );
        if i % 11 == 10 {
            f.remove(&format!("src/m{}.ts", (n + 3) % 7));
        }
        shas.push(f.commit(&format!("c{i}")));
    }
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let full_ws = ws(&d.path().join("full"), &[&f]);
    run(&full_ws, &mut cache);
    let inc = ws(&d.path().join("inc"), &[&f]);
    set_tip(&f, &shas, 20);
    run(&inc, &mut cache);
    set_tip(&f, &shas, 120);
    let s = run(&inc, &mut cache);
    assert_eq!((s.steps, s.replayed_from), (100, None));
    assert_eq!(
        fingerprint(&inc.index_path()),
        fingerprint(&full_ws.index_path())
    );

    // A force-push: back to c90, then a different commit.
    set_tip(&f, &shas, 91);
    f.git(&["reset", "-q", "--hard"]);
    f.write("src/new.ts", "import { a } from './m1';\n");
    f.commit("rewritten");
    let s = run(&inc, &mut cache);
    assert_eq!(s.rewritten, ["repo"]);
    assert_eq!(s.replayed_from, Some(90));
    let fresh = ws(&d.path().join("fresh"), &[&f]);
    run(&fresh, &mut cache);
    assert_eq!(
        fingerprint(&inc.index_path()),
        fingerprint(&fresh.index_path())
    );
}

/// IDX-14: a folder added to the workspace is walked from its root and merged by the same rule;
/// a folder removed has its open intervals closed by one synthetic step and is marked removed.
#[test]
fn a_folder_added_then_removed_is_picked_up_by_the_next_sync() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let (api, _, web, _) = two_repos(&d.path().join("src"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let both = ws(&d.path().join("both"), &[&api, &web]);
    run(&both, &mut cache);
    let mut grow = ws(&d.path().join("grow"), &[&api]);
    run(&grow, &mut cache);
    grow.repos = both.repos.clone();
    let s = run(&grow, &mut cache);
    assert_eq!(
        (s.added.as_slice(), s.replayed_from),
        (&["web".to_owned()][..], Some(-1))
    );
    assert_eq!(
        fingerprint(&grow.index_path()),
        fingerprint(&both.index_path())
    );

    grow.repos.retain(|r| r.name == "api");
    let s = run(&grow, &mut cache);
    assert_eq!(
        (s.removed.as_slice(), s.steps),
        (&["web".to_owned()][..], 1)
    );
    let ix = Index::open_read(&grow.index_path()).unwrap();
    assert_eq!(
        strings(&ix, "SELECT name FROM repo WHERE removed = 1"),
        ["web"]
    );
    let open_in_web: i64 = ix
        .conn
        .query_row(
            "SELECT count(*) FROM interval i JOIN link l ON l.id = i.link
             JOIN node n ON n.id = l.src JOIN file f ON f.id = n.file
             JOIN repo r ON r.id = f.repo WHERE r.name = 'web' AND i.to_seq IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open_in_web, 0);
    // Idempotent: nothing left to remove.
    assert_eq!(run(&grow, &mut cache).steps, 0);
}

/// D2 / IDX-15: ten concurrent syncs give one writer and nine exits within 50 ms, and the index
/// ends at HEAD even though commits land during the run (each followed by a sync, as a hook does).
#[test]
fn ten_concurrent_syncs_one_writer_and_the_index_ends_at_head() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    for i in 0..60 {
        f.write(
            &format!("src/f{}.ts", i % 9),
            &format!("import './f{}';\n// {i}\n", (i + 1) % 9),
        );
        f.commit(&format!("c{i}"));
    }
    let w = ws(&d.path().join("ws"), &[&f]);
    let cache_path = d.path().join("facts.db");
    let barrier = Barrier::new(11);
    let outcomes = std::thread::scope(|sc| {
        let syncers: Vec<_> = (0..10)
            .map(|_| {
                sc.spawn(|| {
                    let mut cache = FactCache::open(&cache_path).unwrap();
                    barrier.wait();
                    let t = Instant::now();
                    let o = sync(&w, &opts(), &mut cache).unwrap();
                    (matches!(o, SyncOutcome::Synced(_)), t.elapsed())
                })
            })
            .collect();
        let committer = sc.spawn(|| {
            let mut cache = FactCache::open(&cache_path).unwrap();
            barrier.wait();
            for i in 0..5 {
                f.write("src/late.ts", &format!("import './f{i}';\n"));
                f.commit(&format!("late {i}"));
                sync(&w, &opts(), &mut cache).unwrap();
            }
        });
        committer.join().unwrap();
        syncers
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let writers = outcomes.iter().filter(|(ran, _)| *ran).count();
    assert_eq!(writers, 1, "{outcomes:?}");
    for (_, t) in outcomes.iter().filter(|(ran, _)| !ran) {
        assert!(*t < Duration::from_millis(50), "a busy sync took {t:?}");
    }
    let r = freshness::check(&w);
    assert_eq!((r.state, r.repos[0].behind), (State::Current, 0), "{r:?}");
    assert!(!w.dirty_path().exists());
}

/// IDX-15: each state of spec §7, and the behind count with its cap.
#[test]
fn freshness_reports_each_state() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("a.ts", "export {};\n");
    let first = f.commit("one");
    let w = ws(&d.path().join("ws"), &[&f]);
    assert_eq!(freshness::check(&w).state, State::None);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let r = freshness::check(&w);
    assert_eq!((r.state, r.repos[0].behind), (State::Current, 0));

    f.write("b.ts", "export {};\n");
    f.commit("two");
    let r = freshness::check(&w);
    assert_eq!(
        (r.state, r.repos[0].behind, r.repos[0].rewritten),
        (State::Behind, 1, false)
    );
    // The memo answers a burst with one check.
    let memo = freshness::freshness(&w);
    f.write("c.ts", "export {};\n");
    f.commit("three");
    assert_eq!(freshness::freshness(&w), memo);

    {
        let _held = WriterLock::try_acquire(&w.dir).unwrap().unwrap();
        assert_eq!(freshness::check(&w).state, State::Syncing);
    }

    // A force-push away from the indexed commit.
    run(&w, &mut cache);
    f.git(&["update-ref", "refs/heads/main", &first]);
    f.git(&["reset", "-q", "--hard"]);
    f.write("d.ts", "export {};\n");
    f.commit("rewritten");
    let r = freshness::check(&w);
    assert_eq!((r.state, r.repos[0].rewritten), (State::Rewritten, true));
    run(&w, &mut cache);
    assert_eq!(freshness::check(&w).state, State::Current);

    // Over the cap: 1,000+.
    fast_import_commits(&f, 1_005);
    let r = freshness::check(&w);
    // Stopped by the 1,000-step cap or the 20 ms budget, whichever comes first.
    assert_eq!(
        (r.state, r.repos[0].behind_capped, r.repos[0].rewritten),
        (State::Behind, true, false)
    );
    assert!(r.repos[0].behind <= BEHIND_CAP);

    let set_meta = |k: &str, v: &dyn cartograph_store::rusqlite::ToSql| {
        let c = cartograph_store::rusqlite::Connection::open(w.index_path()).unwrap();
        c.execute("UPDATE meta SET value = ?2 WHERE key = ?1", (k, v))
            .unwrap();
    };
    set_meta("state", &"syncing");
    assert_eq!(freshness::check(&w).state, State::Failed);
    set_meta("state", &"complete");
    set_meta("format_version", &1);
    assert_eq!(freshness::check(&w).state, State::NeedsMigrate);
}

/// Appends `n` commits to `f`'s branch in one `git fast-import`.
fn fast_import_commits(f: &Fixture, n: usize) {
    use std::io::Write;
    let head = f.git(&["rev-parse", "HEAD"]);
    let mut s = String::new();
    for i in 0..n {
        let t = T0 + 100_000 + i as i64;
        s += &format!(
            "commit refs/heads/main\ncommitter A <a@x> {t} +0000\ndata 1\nx\n{}M 100644 inline gen.ts\ndata 2\n{}\n\n",
            if i == 0 {
                format!("from {head}\n")
            } else {
                String::new()
            },
            i % 10
        );
    }
    s += "done\n";
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(&f.dir)
        .args(["fast-import", "--quiet", "--done"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

/// IDX-15: request_sync never blocks, respects auto_sync = "never", leaves the dirty flag when a
/// sync holds the lock, and starts at most one child per workspace per 2 s.
#[cfg(unix)]
#[test]
fn request_sync_spawns_once_and_never_when_disabled() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let w = Workspace {
        dir: d.path().join("ws"),
        ..Default::default()
    };
    let exe = Path::new("/usr/bin/true");
    let r = request_sync_with(&w, AutoSync::Never, exe, d.path()).unwrap();
    assert_eq!(r, SyncRequest::Disabled);
    assert!(!w.dir.exists(), "never touches anything");
    std::fs::create_dir_all(&w.dir).unwrap();
    {
        let _held = WriterLock::try_acquire(&w.dir).unwrap().unwrap();
        let r = request_sync_with(&w, AutoSync::Background, exe, d.path()).unwrap();
        assert_eq!(r, SyncRequest::AlreadyRunning);
        assert!(w.dirty_path().exists());
    }
    let t = Instant::now();
    assert_eq!(
        request_sync_with(&w, AutoSync::Background, exe, d.path()).unwrap(),
        SyncRequest::Spawned
    );
    assert_eq!(
        request_sync_with(&w, AutoSync::Background, exe, d.path()).unwrap(),
        SyncRequest::RateLimited
    );
    assert!(t.elapsed() < Duration::from_millis(500));
}

/// The D1 benchmark: 30 repos, one of them 40 commits behind, under 100 ms at p95 warm.
#[test]
fn freshness_of_thirty_repos_is_under_100_ms() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut fixtures = Vec::new();
    for i in 0..30 {
        let mut f = Fixture::init(&d.path().join(format!("r{i:02}")));
        f.write("a.ts", &format!("export const r = {i};\n"));
        f.commit("one");
        fixtures.push(f);
    }
    let refs: Vec<&Fixture> = fixtures.iter().collect();
    let w = ws(&d.path().join("ws"), &refs);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    fast_import_commits(&fixtures[7], 40);
    let mut times: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            let r = freshness::check(&w);
            assert_eq!(r.state, State::Behind);
            // The count has a 20 ms budget: on a loaded machine it stops early and says so
            // (`behind_capped`, shown as "N+"); otherwise it is exact.
            let r7 = &r.repos[7];
            assert!(
                r7.behind == 40 || (r7.behind_capped && r7.behind < 40),
                "behind {} capped {}",
                r7.behind,
                r7.behind_capped
            );
            t.elapsed()
        })
        .collect();
    times.sort();
    let p95 = times[18];
    println!("freshness, 30 repos: median {:?}, p95 {p95:?}", times[10]);
    assert!(p95 < Duration::from_millis(100), "p95 {p95:?}");
}

/// The IDX-12 gate, measured on acme-scale (9 repos, 35,439 commits; the research's folder of 43
/// repos and 14,730 commits isn't available): one timeline within 8.0 s.
#[test]
#[ignore = "scale gate: run in release with --ignored"]
fn acme_scale_on_one_timeline() {
    let _serial = serial();
    let root = cartograph_fixtures::acme_scale();
    let d = tempfile::tempdir().unwrap();
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: cartograph_fixtures::acme::repo_names()
            .into_iter()
            .map(|n| RepoSpec {
                path: root.join(&n),
                name: n,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let cold = run(&w, &mut cache);
    println!("acme-scale cold: {cold:?}");
    let warm_ws = Workspace {
        dir: d.path().join("warm"),
        ..w.clone()
    };
    let t = Instant::now();
    let warm = run(&warm_ws, &mut cache);
    println!("acme-scale warm: {} ms, {warm:?}", t.elapsed().as_millis());
    assert_eq!(warm.steps, cartograph_fixtures::acme::COMMITS);
    assert!(
        t.elapsed() < Duration::from_secs(8),
        "warm {:?}",
        t.elapsed()
    );
}

/// A bare clone sharing `src`'s objects, so its branch can be moved freely.
fn shared_clone(src: &Path, dst: &Path) -> PathBuf {
    let ok = std::process::Command::new("git")
        .args(["clone", "--quiet", "--bare", "--shared"])
        .arg(src)
        .arg(dst)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap()
        .success();
    assert!(ok);
    dst.to_owned()
}

/// The IDX-14 gate: 100 new commits on an indexed history, warm. On remotion when
/// CARTOGRAPH_REMOTION is set, else on the largest acme-scale repo.
#[test]
#[ignore = "scale gate: run in release with --ignored"]
fn a_hundred_new_commits_warm() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let (src, rev) = match std::env::var_os("CARTOGRAPH_REMOTION") {
        Some(p) => (PathBuf::from(p), "2b1b0c9".to_owned()),
        None => (
            cartograph_fixtures::acme_scale().join("acme-01"),
            "HEAD".to_owned(),
        ),
    };
    let clone = shared_clone(&src, &d.path().join("repo"));
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&clone)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    let tip = git(&["rev-parse", &format!("{rev}^{{commit}}")]);
    let base = git(&["rev-parse", &format!("{tip}~100")]);
    git(&["symbolic-ref", "HEAD", "refs/heads/cartograph-gate"]);
    git(&["update-ref", "refs/heads/cartograph-gate", &base]);
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: clone.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let first = run(&w, &mut cache);
    println!("first sync: {} steps in {} ms", first.steps, first.total_ms);
    git(&["update-ref", "refs/heads/cartograph-gate", &tip]);
    let t = Instant::now();
    let s = run(&w, &mut cache);
    let ms = t.elapsed().as_millis();
    println!(
        "100 new commits: {ms} ms (rehydrate {}, walk {}, extract+replay+write {}); {s:?}",
        s.rehydrate_ms, s.walk_ms, s.replay_ms
    );
    assert_eq!(s.steps, 100);
}

/// The manifest's branch, scopes and excludes decide what is walked; the workspace id lands in
/// meta; a changed exclude replays from the start and equals a fresh sync.
#[test]
fn branch_scopes_and_excludes_decide_what_is_indexed() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("platform"));
    f.write(
        "packages/auth/package.json",
        r#"{"name":"auth","main":"src/a.ts"}"#,
    );
    f.write("packages/auth/src/a.ts", "import './b';\n");
    f.write("packages/auth/src/b.ts", "export {};\n");
    f.write("packages/auth/dist/out.ts", "import '../src/a';\n");
    f.write("apps/web/w.ts", "import 'auth';\n");
    f.commit("main");
    // A feature branch checked out: sync still indexes main.
    f.git(&["checkout", "-q", "-b", "feature"]);
    f.write("packages/auth/src/c.ts", "export {};\n");
    f.commit("feature work");
    let mut w = Workspace {
        dir: d.path().join("ws").join("7f3a91c2"),
        repos: vec![RepoSpec {
            name: "platform".into(),
            path: f.dir.clone(),
            branch: Some("main".into()),
            scopes: vec!["packages/auth".into()],
        }],
        exclude: vec!["**/dist".into()],
        config_hash: Some("sha256:test".into()),
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    assert_eq!(run(&w, &mut cache).steps, 1);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        strings(&ix, "SELECT path FROM file_path ORDER BY path"),
        [
            "packages/auth/package.json",
            "packages/auth/src/a.ts",
            "packages/auth/src/b.ts"
        ]
    );
    assert_eq!(
        strings(
            &ix,
            "SELECT CAST(value AS TEXT) FROM meta WHERE key IN ('workspace_id', 'config_hash') ORDER BY key"
        ),
        ["sha256:test", "7f3a91c2"]
    );
    drop(ix);
    let r = freshness::check(&w);
    assert_eq!(
        (
            r.state,
            r.repos[0].behind,
            r.repos[0].on_branch_other_than_indexed
        ),
        (State::Current, 0, true)
    );

    w.exclude.clear();
    w.repos[0].scopes.clear();
    let s = run(&w, &mut cache);
    assert_eq!(s.replayed_from, Some(-1));
    let fresh = Workspace {
        dir: d.path().join("fresh"),
        ..w.clone()
    };
    run(&fresh, &mut cache);
    assert_eq!(
        fingerprint(&w.index_path()).0,
        fingerprint(&fresh.index_path()).0
    );
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(strings(&ix, "SELECT count(*) || '' FROM file_path"), ["4"]);
}

#[test]
fn a_branch_only_origin_has_is_indexed_from_its_remote_tracking_ref() {
    let _s = serial();
    let t = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&t.path().join("app"));
    f.write("src/a.ts", "export const a = 1;\n");
    f.commit("one");
    f.write("src/b.ts", "import { a } from './a';\n");
    f.commit("two");
    // As after `git clone -b main` of a repo whose default is trunk: origin/HEAD names trunk, and
    // only origin/trunk exists. The ladder picks trunk, so sync must walk it, not skip the repo.
    f.git(&["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
    let mut w = ws(&t.path().join("data"), &[&f]);
    w.repos[0].branch = Some("trunk".into());
    let mut cache = FactCache::open(&t.path().join("facts.db")).unwrap();
    assert_eq!(run(&w, &mut cache).steps, 2);
    assert_eq!(freshness::check(&w).state, State::Current);
}

#[test]
fn one_repository_never_reports_packages_declared_by_more_than_one_repo() {
    let _s = serial();
    let t = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&t.path().join("mono"));
    // Two packages with one name in one repo: its top folders aren't repos.
    f.write("apps/web/package.json", r#"{"name":"web"}"#);
    f.write("test/fixtures/web/package.json", r#"{"name":"web"}"#);
    f.commit("one");
    let w = ws(&t.path().join("data"), &[&f]);
    let mut cache = FactCache::open(&t.path().join("facts.db")).unwrap();
    assert_eq!(run(&w, &mut cache).duplicate_packages, Vec::<String>::new());
}

#[test]
fn a_cancel_stops_the_round_after_its_batch_and_the_next_sync_carries_on() {
    let _s = serial();
    // The first acme-scale repo: thousands of steps, so the round writes several batches.
    let repo = cartograph_fixtures::acme_scale().join(&cartograph_fixtures::acme::repo_names()[0]);
    let t = tempfile::tempdir().unwrap();
    let w = Workspace {
        dir: t.path().join("ws"),
        repos: vec![RepoSpec {
            name: "app".into(),
            path: repo,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&t.path().join("facts.db")).unwrap();
    // DELETE /jobs lands while the first round replays (a first index is one round).
    let dir = w.dir.clone();
    let mut asked = false;
    let out =
        cartograph_index::sync::sync_with_progress(&w, &opts(), &mut cache, &mut |done, _| {
            if !asked && done > 0 {
                asked = true;
                cartograph_store::writer::request_cancel(&dir).unwrap();
            }
        })
        .unwrap();
    let SyncOutcome::Synced(first) = out else {
        panic!("nothing else is running")
    };
    // Stopped after the batch the cancel landed in, as a complete index that is behind.
    assert!(first.steps > 0);
    assert_eq!(freshness::check(&w).state, State::Behind);
    let rest = run(&w, &mut cache);
    assert!(rest.steps > 0, "the cancelled round stopped before the end");
    assert_eq!(
        rest.replayed_from, None,
        "nothing written before the stop is replayed"
    );
    assert_eq!(freshness::check(&w).state, State::Current);
}
