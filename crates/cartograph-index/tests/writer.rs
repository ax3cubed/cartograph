//! The writer's crash safety (IDX-13), rebuild by rename and migrations (IDX-16), and the fact hash
//! (IDX-17), on the generated fixtures.
//!
//! The kill loop runs this test binary again as a child that rebuilds an index, and SIGKILLs it at
//! random points: 100 iterations here (`CARTOGRAPH_KILLS` raises it; HD-08 runs 1,000 nightly).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cartograph_facts::FactCache;
use cartograph_index::{build_single, rebuild_single};
use cartograph_store::migrate::{self, new_path};
use cartograph_store::rusqlite::Connection;
use cartograph_store::writer::WriterLock;
use cartograph_store::{FactHash, Index, SCHEMA, fact_hash};
use proptest::prelude::*;

const ROLE: &str = "CARTOGRAPH_TEST_REBUILD";

fn hash_of(p: &Path) -> FactHash {
    fact_hash(&Index::open_read(p).unwrap().conn).unwrap()
}

fn build(repo: &Path, to: Option<&str>, out: &Path) {
    let mut cache = FactCache::open(&out.with_extension("facts.db")).unwrap();
    build_single(repo, to, out, &mut cache, 2).unwrap();
}

fn meta(conn: &Connection, key: &str) -> String {
    conn.query_row(
        "SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .unwrap()
}

/// The child of the kill loop: rebuilds `<dir>/index.cartograph` from the repo, under the lock.
#[test]
fn child_rebuild() {
    let Ok(spec) = std::env::var(ROLE) else {
        return;
    };
    let (dir, repo) = spec.split_once('\n').unwrap();
    let dir = Path::new(dir);
    let lock = WriterLock::try_acquire(dir)
        .unwrap()
        .expect("one child at a time");
    let mut cache = FactCache::open(&dir.join("facts.db")).unwrap();
    rebuild_single(
        &lock,
        Path::new(repo),
        None,
        &dir.join("index.cartograph"),
        &mut cache,
        2,
    )
    .unwrap();
}

fn spawn_rebuild(dir: &Path, repo: &Path) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["child_rebuild", "--exact", "--test-threads=1"])
        .env(ROLE, format!("{}\n{}", dir.display(), repo.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The first repo of acme-scale: 3,937 commits, long enough that kills land inside the write.
fn kill_repo() -> PathBuf {
    let root = cartograph_fixtures::acme_scale();
    root.join(&cartograph_fixtures::acme::repo_names()[0])
}

#[test]
fn sigkill_at_random_points_never_damages_the_index() {
    let repo = kill_repo();
    let d = tempfile::tempdir().unwrap();
    let index = d.path().join("index.cartograph");
    // The reference run, uninterrupted; it also warms the fact cache, as a second sync would be.
    let c = spawn_rebuild(d.path(), &repo).wait_with_output().unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let reference = hash_of(&index).to_string();
    // A warm run, timed: when the .new file appears (walk and extract are done) and when the run
    // ends. Kill points are spread over that write, the part that can damage something.
    let t0 = Instant::now();
    let mut c = spawn_rebuild(d.path(), &repo);
    let mut write_from = None;
    while c.try_wait().unwrap().is_none() {
        if write_from.is_none() && new_path(&index).exists() {
            write_from = Some(t0.elapsed());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let t = t0.elapsed();
    let from = write_from.expect("the rebuild writes a .new file");
    let kills: u32 = std::env::var("CARTOGRAPH_KILLS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(100);
    let mut seed: u64 = 0x5eed_cafe;
    let (mut mid_build, mut finished) = (0, 0);
    // Each kill is timed from when its own run's .new file appears, at a point in the first 80% of
    // the write the timed run measured. A delay counted from the spawn assumed the walk and
    // extract took as long as in the timed run, which load (sibling tests, CI neighbours) changes:
    // the kills then landed after the run had finished.
    let write = t - from;
    for i in 0..kills {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let u = (seed >> 11) as f64 / (1u64 << 53) as f64;
        let at = write.mul_f64(u * 0.8);
        eprintln!("kill {i} {at:?} into the write (the timed write took {write:?})");
        // The last kill's leftover was checked below; this run's .new must be its own.
        let new = new_path(&index);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", new.display()));
        }
        let mut child = spawn_rebuild(d.path(), &repo);
        let waiting = Instant::now();
        while !new.exists()
            && child.try_wait().unwrap().is_none()
            && waiting.elapsed() < Duration::from_secs(300)
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(at);
        let _ = child.kill(); // SIGKILL on Unix
        let status = child.wait().unwrap();
        // Either killed, or it finished first; never a failure of its own.
        assert!(
            status.success() || status.code().is_none(),
            "kill {i}: the child failed by itself"
        );
        finished += u32::from(status.success());
        // The index in place is the complete old one or the complete new one, never a mix.
        let ix = Index::open_read(&index).unwrap_or_else(|e| panic!("kill {i} at {at:?}: {e}"));
        assert_eq!(meta(&ix.conn, "state"), "complete");
        assert_eq!(meta(&ix.conn, "fact_hash"), reference, "kill {i}");
        drop(ix);
        // A half-written .new is itself a consistent database: batches commit whole.
        let tmp = new_path(&index);
        if std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) {
            mid_build += 1;
            let c = Connection::open(&tmp).unwrap();
            let check: String = c
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .unwrap_or_else(|e| e.to_string());
            assert_eq!(check, "ok", "kill {i}: the .new file");
        }
    }
    eprintln!(
        "{kills} kills over the write ({from:?}..{t:?}): {mid_build} left a .new file, \
         {finished} finished first"
    );
    assert!(mid_build > kills / 2, "most kills land inside the write");
    // The next run deletes the leftover and ends with the same fact hash.
    assert!(spawn_rebuild(d.path(), &repo).wait().unwrap().success());
    assert!(!new_path(&index).exists());
    assert_eq!(hash_of(&index).to_string(), reference);
}

#[test]
fn a_reader_keeps_the_old_index_through_a_rebuild() {
    let repo = cartograph_fixtures::northwind();
    let d = tempfile::tempdir().unwrap();
    let index = d.path().join("index.cartograph");
    let lock = WriterLock::try_acquire(d.path()).unwrap().unwrap();
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    rebuild_single(&lock, &repo, Some("HEAD~5"), &index, &mut cache, 2).unwrap();
    let commits = |c: &Connection| -> i64 {
        c.query_row("SELECT count(*) FROM commits", [], |r| r.get(0))
            .unwrap()
    };
    let old = Index::open_read(&index).unwrap();
    old.conn.execute_batch("BEGIN").unwrap();
    let before = commits(&old.conn);
    // A leftover from a killed run is deleted, not built on.
    std::fs::write(new_path(&index), b"half a file").unwrap();
    rebuild_single(&lock, &repo, None, &index, &mut cache, 2).unwrap();
    assert_eq!(commits(&old.conn), before, "the open reader still answers");
    old.conn.execute_batch("COMMIT").unwrap();
    assert_eq!(commits(&old.conn), before, "it still has the old file");
    let new = Index::open_read(&index).unwrap();
    assert_eq!(
        commits(&new.conn),
        before + 5,
        "the next open sees the new index"
    );
    // A failed build leaves the index as it was.
    let e = rebuild_single(&lock, &d.path().join("nope"), None, &index, &mut cache, 2);
    assert!(e.is_err());
    assert!(!new_path(&index).exists());
    assert_eq!(commits(&Index::open_read(&index).unwrap().conn), before + 5);
}

#[test]
fn a_v1_index_migrates_to_v2_with_the_fresh_build_s_fact_hash() {
    let repo = cartograph_fixtures::northwind();
    let d = tempfile::tempdir().unwrap();
    let fresh = d.path().join("fresh.cartograph");
    build(&repo, None, &fresh);
    // The v1 fixture: the same build with v2's columns taken away and v1's numbers.
    let v1 = d.path().join("index.cartograph");
    std::fs::copy(&fresh, &v1).unwrap();
    Connection::open(&v1)
        .unwrap()
        .execute_batch(&format!(
            "{}
             ALTER TABLE repo DROP COLUMN indexed_commit; ALTER TABLE repo DROP COLUMN indexed_seq;
             ALTER TABLE repo DROP COLUMN removed;
             DROP INDEX commits_repo_sha; DROP INDEX commits_sha;
             CREATE UNIQUE INDEX commits_sha ON commits (sha);
             UPDATE meta SET value = 1 WHERE key = 'format_version';
             PRAGMA user_version = 1;",
            migrate::UNDO_V3
        ))
        .unwrap();
    assert_eq!(
        Index::open_read(&v1).unwrap().state,
        cartograph_store::State::NeedsMigrate
    );
    let lock = WriterLock::try_acquire(d.path()).unwrap().unwrap();
    assert!(migrate::migrate(&lock, &v1).unwrap());
    let ix = Index::open_write(&v1).unwrap();
    assert_eq!(fact_hash(&ix.conn).unwrap(), hash_of(&fresh));
}

/// A copy of `src`'s rows into a new file with another page size, other row ids and seq numbers,
/// another local path and the reverse insert order: what another machine could build.
fn rewrite(src: &Path, out: &Path, page_size: u32, shift: i64, reverse: bool) {
    let c = Connection::open(out).unwrap();
    c.execute_batch(&format!(
        "PRAGMA page_size = {page_size}; PRAGMA application_id = {};
         PRAGMA user_version = {};\n{SCHEMA}",
        cartograph_store::APPLICATION_ID,
        cartograph_store::USER_VERSION
    ))
    .unwrap();
    c.execute("ATTACH ?1 AS s", [src.to_string_lossy()])
        .unwrap();
    let order = if reverse { "DESC" } else { "ASC" };
    c.execute_batch(&format!(
        "INSERT INTO meta SELECT * FROM s.meta;
         INSERT INTO extractor SELECT id + {shift}, name, version FROM s.extractor ORDER BY id {order};
         INSERT INTO repo SELECT id + {shift}, name, '/elsewhere/' || name, head, shallow,
           indexed_commit, indexed_seq + {shift}, removed FROM s.repo ORDER BY id {order};
         INSERT INTO run (id, started_at, finished_at, writer_version, head, stats)
           SELECT id, started_at + 999, finished_at + 999, writer_version, head, stats FROM s.run;
         INSERT INTO commits SELECT seq + {shift}, repo + {shift}, sha, time, corrected_time, author,
           summary FROM s.commits ORDER BY seq {order};
         INSERT INTO file SELECT id + {shift}, repo + {shift} FROM s.file ORDER BY id {order};
         INSERT INTO file_path SELECT file + {shift}, path, from_seq + {shift}, to_seq + {shift},
           via, similarity FROM s.file_path ORDER BY rowid {order};
         INSERT INTO file_version SELECT file + {shift}, blob, from_seq + {shift}, to_seq + {shift}
           FROM s.file_version ORDER BY rowid {order};
         INSERT INTO fact SELECT blob, extractor + {shift}, payload FROM s.fact ORDER BY blob {order};
         INSERT INTO node (id, kind, file, key) SELECT id + {shift}, kind, file + {shift}, key
           FROM s.node ORDER BY id {order};
         INSERT INTO link SELECT id + {shift}, kind, src + {shift}, dst + {shift} FROM s.link
           ORDER BY id {order};
         INSERT INTO interval SELECT link + {shift}, from_seq + {shift}, to_seq + {shift}
           FROM s.interval ORDER BY rowid {order};
         DETACH s;"
    ))
    .unwrap();
}

fn northwind_index(dir: &Path) -> PathBuf {
    let p = dir.join("index.cartograph");
    build(&cartograph_fixtures::northwind(), None, &p);
    p
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(6))]
    #[test]
    fn the_hash_ignores_page_size_row_ids_seq_numbers_and_insert_order(
        page in prop::sample::select(vec![512u32, 1024, 4096, 65536]),
        shift in 1i64..10_000,
        reverse in any::<bool>(),
    ) {
        let d = tempfile::tempdir().unwrap();
        let src = northwind_index(d.path());
        let out = d.path().join("other.cartograph");
        rewrite(&src, &out, page, shift, reverse);
        let page_of = |p: &Path| -> u32 {
            Connection::open(p).unwrap().query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap()
        };
        prop_assert_eq!(page_of(&out), page);
        prop_assert_eq!(hash_of(&out), hash_of(&src));
    }
}

#[test]
fn changing_one_row_changes_the_hash_and_names_the_section() {
    let d = tempfile::tempdir().unwrap();
    let src = northwind_index(d.path());
    let h = hash_of(&src);
    assert_eq!(
        meta(&Index::open_read(&src).unwrap().conn, "fact_hash"),
        h.to_string(),
        "the build stores its hash"
    );
    let cases = [
        // Northwind's links all stay open to head: move one's start, and close one.
        (
            "UPDATE interval SET from_seq = from_seq + 1 WHERE rowid = (SELECT min(rowid)
               FROM interval WHERE from_seq < (SELECT max(seq) FROM commits))",
            "repo 0 (northwind) links",
        ),
        (
            "UPDATE interval SET to_seq = (SELECT max(seq) FROM commits) WHERE rowid = (SELECT
               min(rowid) FROM interval WHERE from_seq < (SELECT max(seq) FROM commits))",
            "repo 0 (northwind) links",
        ),
        (
            "UPDATE fact SET payload = CAST(payload || x'00' AS BLOB) WHERE blob = (SELECT min(blob) FROM fact)",
            "facts",
        ),
        (
            "UPDATE file_path SET path = path || 'x' WHERE rowid = 1",
            "repo 0 (northwind) files",
        ),
        (
            "UPDATE extractor SET version = version || '.1' WHERE id = 1",
            "extractors",
        ),
        (
            "INSERT INTO meta (key, value) VALUES ('config_hash', 'sha256:00')",
            "config",
        ),
        (
            "UPDATE repo SET indexed_commit = NULL",
            "repo 0 (northwind) indexed commit",
        ),
    ];
    for (i, (sql, section)) in cases.iter().enumerate() {
        let p = d.path().join(format!("m{i}.cartograph"));
        std::fs::copy(&src, &p).unwrap();
        let c = Connection::open(&p).unwrap();
        c.execute_batch(sql).unwrap();
        assert_eq!(c.changes(), 1, "{sql}");
        drop(c);
        let m = hash_of(&p);
        assert_ne!(m.digest(), h.digest(), "{sql}");
        assert_eq!(h.first_difference(&m).as_deref(), Some(*section), "{sql}");
    }
    let v = cartograph_store::hash::verify(&src, &d.path().join("m0.cartograph")).unwrap();
    assert_eq!(
        v.first_difference.as_deref(),
        Some("repo 0 (northwind) links")
    );
    let same = cartograph_store::hash::verify(&src, &src).unwrap();
    assert_eq!(
        (same.first_difference, same.ours.clone()),
        (None, same.theirs)
    );
}

#[test]
fn every_batch_of_a_build_bumps_the_generation() {
    // 3,937 commits: four batches of up to 1,000 steps, and the final commit.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("index.cartograph");
    build(&kill_repo(), None, &p);
    let g: i64 = meta(&Index::open_read(&p).unwrap().conn, "generation")
        .parse()
        .unwrap();
    assert!(g >= 4, "generation {g}");
}

#[test]
#[ignore = "timing: run in release"]
fn fact_hash_timing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("index.cartograph");
    build(&kill_repo(), None, &p);
    let ix = Index::open_read(&p).unwrap();
    let t = Instant::now();
    let h = fact_hash(&ix.conn).unwrap();
    eprintln!("fact hash of acme-01 in {:?}: {h}", t.elapsed());
    assert!(t.elapsed() < Duration::from_secs(2));
}
