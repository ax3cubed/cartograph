//! Fixture repositories and their golden indexes (HD-03). Test support only: nothing in the product
//! depends on this crate.
//!
//! Each fixture is a real git repository (or a folder of them) built by a seeded generator with
//! fixed authors and dates, so the same generator gives the same commit ids on every OS. Fixtures
//! are generated, never downloaded, and cached under the OS temp folder by a hash of the generator's
//! own source; a changed generator builds a new folder.
//!
//! Getting a fixture index in another test (add `cartograph-fixtures` as a dev-dependency):
//!
//! ```no_run
//! let dir = tempfile::tempdir().unwrap();
//! let index = cartograph_fixtures::index(&cartograph_fixtures::northwind(), dir.path());
//! // `index` is a complete index.cartograph file; open it with cartograph_store::Index::open_read.
//! ```
//!
//! Goldens live in `fixtures/golden/` as insta snapshots: the fact hash and the fact list. Any
//! change to an extractor, the replay or the generators shows up there as a diff. Review it with
//! `cargo insta review`, or accept every change with
//! `INSTA_UPDATE=always cargo test -p cartograph-fixtures`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cartograph_facts::FactCache;
use cartograph_parse::{BlobFacts, Fact};
use cartograph_store::Index;
use cartograph_store::rusqlite::{self, Row};
use sha2::{Digest, Sha256};

pub mod acme;
pub mod history;
pub mod layout;
pub mod northwind;
pub mod scale;

const FIXTURE_RS: &str = include_str!("../../../crates/cartograph-git/src/fixture.rs");
const LIB_RS: &str = include_str!("lib.rs");

/// The `northwind` repository: the demo's 6 services and 22 commits.
pub fn northwind() -> PathBuf {
    cached(
        "northwind",
        &[northwind::SOURCE, northwind::WORKSPACE_TS],
        northwind::generate,
        |_| {},
    )
    .join("northwind")
}

/// The `history` repository: renames, a force-push, a rebase, a merge, a deleted and re-added file.
pub fn history() -> PathBuf {
    cached("history", &[history::SOURCE], history::generate, |_| {}).join("history")
}

/// The `layout` folder: a pnpm monorepo, go.work, a submodule, worktrees, a shallow clone, a bare
/// repo and a `.code-workspace` file. Not a repository itself.
pub fn layout() -> PathBuf {
    cached(
        "layout",
        &[layout::SOURCE],
        layout::generate,
        layout::after_move,
    )
}

/// The `acme-scale` folder: 9 repositories at board 00's numbers.
pub fn acme_scale() -> PathBuf {
    cached("acme-scale", &[acme::SOURCE], acme::generate, |_| {})
}

/// Builds the index of `repo` into `dir/index.cartograph`, with a fresh fact cache in `dir`, so a
/// stale cache entry can never hide an extractor change. Returns the index path.
pub fn index(repo: &Path, dir: &Path) -> PathBuf {
    let out = dir.join("index.cartograph");
    let mut cache = FactCache::open(&dir.join("facts.db")).expect("a fact cache");
    cartograph_index::build_single(
        repo,
        None,
        &out,
        &mut cache,
        cartograph_index::default_workers(),
    )
    .unwrap_or_else(|e| panic!("indexing {}: {e}", repo.display()));
    out
}

/// The canonical fact list of an index: extractors, commits, file paths and versions, facts and
/// link intervals, one record a line. Commits are named by sha and nothing depends on row ids,
/// seq numbers, run times or local paths, so two machines give the same list.
pub fn fact_list(index: &Path) -> Vec<String> {
    let ix = Index::open_read(index).expect("the index opens");
    let c = &ix.conn;
    let q = |sql: &str, f: &dyn Fn(&Row) -> rusqlite::Result<String>| -> Vec<String> {
        let mut st = c.prepare(sql).expect("valid sql");
        let mut v: Vec<String> = st
            .query_map([], |r| f(r))
            .expect("query runs")
            .map(|r| r.expect("row"))
            .collect();
        v.sort();
        v
    };
    let s = |r: &Row, i: usize| r.get::<_, String>(i);
    let o = |r: &Row, i: usize| -> rusqlite::Result<String> {
        Ok(r.get::<_, Option<String>>(i)?.unwrap_or_else(|| "-".into()))
    };
    let mut out = q("SELECT name, version FROM extractor", &|r| {
        Ok(format!("extractor {} {}", s(r, 0)?, s(r, 1)?))
    });
    // First-parent order is part of the history, so commits keep it (seq is only the sort key).
    let mut commits = c
        .prepare("SELECT lower(hex(sha)), author, summary FROM commits ORDER BY seq")
        .expect("valid sql");
    out.extend(
        commits
            .query_map([], |r| {
                Ok(format!("commit {} {}: {}", s(r, 0)?, s(r, 1)?, s(r, 2)?))
            })
            .expect("query runs")
            .map(|r| r.expect("row")),
    );
    const SHA: &str = "(SELECT substr(lower(hex(sha)), 1, 12) FROM commits WHERE seq = ";
    out.extend(q(
        &format!("SELECT path, {SHA}from_seq), {SHA}to_seq) FROM file_path"),
        &|r| Ok(format!("path {} {} {}", s(r, 0)?, s(r, 1)?, o(r, 2)?)),
    ));
    out.extend(q(
        &format!(
            "SELECT fp.path, substr(lower(hex(fv.blob)), 1, 12), {SHA}fv.from_seq), {SHA}fv.to_seq)
             FROM file_version fv JOIN file_path fp ON fp.file = fv.file
               AND fp.from_seq <= fv.from_seq AND (fp.to_seq IS NULL OR fp.to_seq > fv.from_seq)"
        ),
        &|r| {
            Ok(format!(
                "version {} {} {} {}",
                s(r, 0)?,
                s(r, 1)?,
                s(r, 2)?,
                o(r, 3)?
            ))
        },
    ));
    let mut st = c
        .prepare(
            "SELECT substr(lower(hex(f.blob)), 1, 12), e.name, f.payload
             FROM fact f JOIN extractor e ON e.id = f.extractor",
        )
        .expect("valid sql");
    let mut facts: Vec<String> = st
        .query_map([], |r| Ok((s(r, 0)?, s(r, 1)?, r.get::<_, Vec<u8>>(2)?)))
        .expect("query runs")
        .flat_map(|r| {
            let (blob, kind, payload) = r.expect("row");
            let head = format!("fact {blob} {kind}");
            match BlobFacts::decode(&payload).expect("facts decode") {
                BlobFacts::Skipped(why) => vec![format!("{head} skipped {why:?}")],
                BlobFacts::Facts(f) if f.is_empty() => vec![format!("{head} none")],
                BlobFacts::Facts(f) => f.iter().map(|f| format!("{head} {}", fact(f))).collect(),
            }
        })
        .collect();
    facts.sort();
    out.extend(facts);
    out.extend(q(
        &format!(
            "SELECT l.kind, ps.path, pd.path, {SHA}i.from_seq), {SHA}i.to_seq)
             FROM interval i JOIN link l ON l.id = i.link
             JOIN node ns ON ns.id = l.src JOIN file_path ps ON ps.file = ns.file
             JOIN node nd ON nd.id = l.dst JOIN file_path pd ON pd.file = nd.file
             GROUP BY i.rowid"
        ),
        &|r| {
            Ok(format!(
                "link {} {} -> {} {} {}",
                s(r, 0)?,
                s(r, 1)?,
                s(r, 2)?,
                s(r, 3)?,
                o(r, 4)?
            ))
        },
    ));
    out
}

fn fact(f: &Fact) -> String {
    match f {
        Fact::Import { spec, style, span } => format!(
            "import {style:?} {spec:?} {}:{}-{}:{}",
            span.start_line, span.start_col, span.end_line, span.end_col
        ),
        Fact::DynamicUnresolved { span } => format!(
            "dynamic {}:{}-{}:{}",
            span.start_line, span.start_col, span.end_line, span.end_col
        ),
        Fact::Manifest { field, value } => format!("manifest {field} = {value}"),
        Fact::Host {
            host,
            port,
            context,
        } => format!("host {host} {port:?} {context:?}"),
    }
}

/// The fact hash of an index (IDX-17, B4), as `meta.fact_hash` and `cartograph verify` show it.
pub fn fact_hash(index: &Path) -> String {
    let ix = Index::open_read(index).expect("the index opens");
    cartograph_store::fact_hash(&ix.conn)
        .expect("the fact hash")
        .to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// Generates a fixture once per generator hash into `<temp>/cartograph-fixtures/<name>-<hash>`.
/// The generator writes into a private folder that is renamed into place, so concurrent test
/// processes never see a half-built fixture; `after_move` fixes anything that recorded the old path.
fn cached(name: &str, sources: &[&str], generate: fn(&Path), after_move: fn(&Path)) -> PathBuf {
    let mut h = Sha256::new();
    for s in [LIB_RS, FIXTURE_RS].iter().chain(sources) {
        h.update(s.as_bytes());
        h.update([0]);
    }
    let key = &hex(&h.finalize())[..16];
    let root = std::env::temp_dir().join("cartograph-fixtures");
    let dir = root.join(format!("{name}-{key}"));
    if dir.join(".done").exists() {
        return dir;
    }
    // Tests call this from parallel threads: one generator per process at a time, and every temp
    // folder name unique (subsec nanos are microseconds on macOS, so two threads could share one).
    static GENERATING: Mutex<()> = Mutex::new(());
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let _one = GENERATING.lock().unwrap_or_else(|e| e.into_inner());
    if dir.join(".done").exists() {
        return dir;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    // ponytail: a crashed run leaves its .tmp folder behind in the OS temp folder.
    let tmp = root.join(format!(
        ".{name}-{key}-{}-{}-{nanos}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp).expect("mkdir");
    generate(&tmp);
    // .done goes in last, after after_move has fixed what recorded the temp path (worktree links):
    // whoever sees it gets a finished fixture.
    let finish = |dir: &Path| {
        after_move(dir);
        std::fs::write(dir.join(".done"), "").expect("write");
    };
    match std::fs::rename(&tmp, &dir) {
        Ok(()) => finish(&dir),
        Err(_) if dir.exists() => {
            // Another process moved its copy in first and is finishing it.
            for _ in 0..600 {
                if dir.join(".done").exists() {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return dir;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            // It died before finishing: take its place.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::rename(&tmp, &dir)
                .unwrap_or_else(|e| panic!("moving {} into place: {e}", tmp.display()));
            finish(&dir);
        }
        Err(e) => panic!("moving {} into place: {e}", tmp.display()),
    }
    dir
}

/// Seeded splitmix64: the generators' only source of randomness.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The `i`th of `parts` near-equal shares of `total`.
pub(crate) fn share(total: usize, parts: usize, i: usize) -> usize {
    total * (i + 1) / parts - total * i / parts
}

/// `to` relative to the folder of `from`, as an import specifier.
pub(crate) fn relative(from: &str, to: &str) -> String {
    let a: Vec<&str> = from.split('/').collect();
    let b: Vec<&str> = to.split('/').collect();
    let dir = &a[..a.len() - 1];
    let common = dir.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let rest = b[common..].join("/");
    match dir.len() - common {
        0 => format!("./{rest}"),
        up => format!("{}{rest}", "../".repeat(up)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate(dir: &Path) {
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.join("f"), "x").unwrap();
    }

    fn after_move(dir: &Path) {
        // Slow on purpose: a caller must never get the fixture before this ran in its final place.
        std::thread::sleep(Duration::from_millis(60));
        std::fs::write(dir.join("moved"), "").unwrap();
    }

    #[test]
    fn parallel_callers_get_one_finished_copy() {
        // A source no other run hashed, so the cache starts cold.
        let unique = format!("{:?}", SystemTime::now());
        let dirs: Vec<PathBuf> = std::thread::scope(|s| {
            let calls: Vec<_> = (0..4)
                .map(|_| s.spawn(|| cached("race-check", &[unique.as_str()], generate, after_move)))
                .collect();
            calls.into_iter().map(|c| c.join().unwrap()).collect()
        });
        for d in &dirs {
            assert_eq!(d, &dirs[0]);
            assert!(d.join("moved").exists(), "finished before .done");
            assert!(d.join(".done").exists());
        }
        let _ = std::fs::remove_dir_all(&dirs[0]);
    }
}
