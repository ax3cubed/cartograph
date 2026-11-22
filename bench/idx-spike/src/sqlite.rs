//! Research "Next": intervals in SQLite. Write time, file size, load time and as-of query time from
//! disk, for remotion's 36,193 intervals and a synthetic 60k-file, 200k-link set.

use std::path::Path;
use std::time::Instant;

use rusqlite::{Connection, params};

use crate::R;

/// A seeded generator, so every run writes the same rows.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA application_id = 1129599572; -- 'CART'
CREATE TABLE file (id INTEGER PRIMARY KEY, path TEXT NOT NULL) STRICT;
CREATE TABLE link (id INTEGER PRIMARY KEY, src INTEGER NOT NULL, dst INTEGER NOT NULL, kind INTEGER NOT NULL) STRICT;
CREATE TABLE interval (link INTEGER NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER) STRICT;
";
const INDEXES: &str = "CREATE INDEX interval_from ON interval (from_seq, to_seq, link);";

/// (files, links, intervals, steps)
fn shape(name: &str) -> (u64, u64, u64, u64) {
    match name {
        // remotion: 9,075 files at head, 36,193 intervals over 9,498 first-parent steps.
        "remotion" => (9_075, 33_000, 36_193, 9_498),
        // the decision's fixture: 60k files, 200k links; a link reopens now and then.
        _ => (60_000, 200_000, 230_000, 10_000),
    }
}

fn write(path: &Path, name: &str) -> R<f64> {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let (files, links, intervals, steps) = shape(name);
    let mut rng = Lcg(42);
    let t = Instant::now();
    let mut db = Connection::open(path)?;
    db.execute_batch(SCHEMA)?;
    let tx = db.transaction()?;
    {
        let mut f = tx.prepare("INSERT INTO file (id, path) VALUES (?1, ?2)")?;
        for i in 0..files {
            f.execute(params![
                i as i64,
                format!("packages/p{}/src/dir{}/file{i}.ts", i % 62, i % 40)
            ])?;
        }
        let mut l = tx.prepare("INSERT INTO link (id, src, dst, kind) VALUES (?1, ?2, ?3, ?4)")?;
        for i in 0..links {
            l.execute(params![
                i as i64,
                rng.below(files) as i64,
                rng.below(files) as i64,
                rng.below(5) as i64
            ])?;
        }
        let mut iv =
            tx.prepare("INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)")?;
        for i in 0..intervals {
            let link = if i < links { i } else { rng.below(links) };
            let from = rng.below(steps);
            // About a third of intervals close; the rest are live at head (research: 67% survive 2 years).
            let to = (rng.below(3) == 0).then(|| (from + 1 + rng.below(steps - from)) as i64);
            iv.execute(params![link as i64, from as i64, to])?;
        }
    }
    tx.commit()?;
    db.execute_batch(INDEXES)?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); ANALYZE;")?;
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

fn as_of(db: &Connection, step: i64) -> R<usize> {
    let mut q = db.prepare_cached(
        "SELECT link FROM interval WHERE from_seq <= ?1 AND (to_seq IS NULL OR to_seq > ?1)",
    )?;
    let mut n = 0;
    let mut rows = q.query([step])?;
    while rows.next()?.is_some() {
        n += 1;
    }
    Ok(n)
}

pub fn bench(dir: &Path, name: &str, runs: usize) -> R<serde_json::Value> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{name}.db"));
    let (_, links, intervals, steps) = shape(name);
    let mut write_ms = Vec::new();
    for _ in 0..runs {
        write_ms.push(write(&path, name)?);
    }
    let size = std::fs::metadata(&path)?.len();

    // Open and load every interval into memory (what the server would cache per generation).
    let mut load_ms = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut q = db.prepare("SELECT link, from_seq, to_seq FROM interval")?;
        let rows: Vec<(i64, i64, Option<i64>)> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        assert_eq!(rows.len() as u64, intervals);
        load_ms.push(t.elapsed().as_secs_f64() * 1e3);
    }

    // As of an arbitrary step, from disk: a fresh read-only connection each time, then the query.
    let mut rng = Lcg(7);
    let (mut cold_ms, mut warm_ms, mut live) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..runs.max(10) {
        let step = rng.below(steps) as i64;
        let t = Instant::now();
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        live.push(as_of(&db, step)?);
        cold_ms.push(t.elapsed().as_secs_f64() * 1e3);
        for _ in 0..10 {
            let step = rng.below(steps) as i64;
            let t = Instant::now();
            as_of(&db, step)?;
            warm_ms.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    Ok(serde_json::json!({
        "set": name, "links": links, "intervals": intervals, "steps": steps,
        "file_bytes": size,
        "write_ms": write_ms,
        "load_all_intervals_ms": load_ms,
        "as_of_new_connection_ms": cold_ms,
        "as_of_warm_ms_p50": crate::median(&warm_ms),
        "as_of_warm_ms_p95": crate::percentile(&warm_ms, 0.95),
        "live_links_returned": live,
        "note": "rows are synthetic with the stated counts; the OS page cache is warm after the write (no purge without root)",
    }))
}
