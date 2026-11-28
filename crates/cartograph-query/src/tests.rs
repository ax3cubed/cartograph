use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use cartograph_api::types::{ErrorCode, GraphParams, LinkKind};
use cartograph_core::RepoId;
use cartograph_facts::{FactCache, from_scratch};
use cartograph_git::Repo;
use cartograph_git::fixture::Fixture;
use cartograph_parse::extractor_for;
use cartograph_store::Index;
use cartograph_store::rusqlite::{Connection, params};

use crate::*;

/// A schema-v1 index with one repo and no history, its writer still open.
fn empty_index(path: &Path, generation: i64) -> Connection {
    let ix = Index::create(path).unwrap();
    ix.conn
        .execute_batch("INSERT INTO repo (id, name, path, shallow) VALUES (1, 'r', 'r', 0)")
        .unwrap();
    ix.conn
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = 'generation'",
            [generation],
        )
        .unwrap();
    // The store's open check reads the file header from the main file, so it must have been
    // checkpointed once, as the indexer does at the end of a build.
    ix.conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    ix.conn
}

/// SRV-02's done-when: 8 readers query while a writer commits batches; none sees SQLITE_BUSY and
/// each snapshot sees one generation (every batch adds 10 commits and bumps the generation once,
/// so a torn read would show a count that doesn't match).
#[test]
fn eight_readers_see_one_generation_each_during_writer_batches() {
    const BATCHES: i64 = 150;
    const PER_BATCH: i64 = 10;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("index.cartograph");
    let writer = empty_index(&path, 0);
    let store = Arc::new(Store::open_path(&path).unwrap());
    let done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(9));

    let readers: Vec<_> = (0..8)
        .map(|_| {
            let (store, done, start) = (store.clone(), done.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                let mut seen = BTreeSet::new();
                let mut reads = 0u32;
                while !done.load(Ordering::Relaxed) {
                    let snap = store.snapshot(Limits::default()).expect("no SQLITE_BUSY");
                    let count = |s: &Snapshot<'_>| -> i64 {
                        s.conn()
                            .query_row("SELECT count(*) FROM commits", [], |r| r.get(0))
                            .unwrap()
                    };
                    let first = count(&snap);
                    std::thread::sleep(Duration::from_micros(200));
                    let again = count(&snap);
                    let generation: i64 = snap
                        .conn()
                        .query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    assert_eq!(
                        generation as u64, snap.generation,
                        "generation moved inside a snapshot"
                    );
                    assert_eq!(first, snap.generation as i64 * PER_BATCH, "a torn read");
                    assert_eq!(first, again, "rows moved inside a snapshot");
                    seen.insert(snap.generation);
                    reads += 1;
                }
                (seen, reads)
            })
        })
        .collect();

    start.wait();
    let mut seq = 0i64;
    for _ in 0..BATCHES {
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        for _ in 0..PER_BATCH {
            writer
                .execute(
                    "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
                     VALUES (?1, 1, ?2, ?1, ?1, 'a', 's')",
                    params![seq, seq.to_be_bytes().to_vec()],
                )
                .unwrap();
            seq += 1;
        }
        writer
            .execute_batch("UPDATE meta SET value = value + 1 WHERE key = 'generation'; COMMIT")
            .unwrap();
        std::thread::sleep(Duration::from_micros(300));
    }
    done.store(true, Ordering::Relaxed);
    let mut generations = BTreeSet::new();
    let mut reads = 0;
    for r in readers {
        let (seen, n) = r.join().expect("a reader failed");
        generations.extend(seen);
        reads += n;
    }
    eprintln!(
        "{reads} snapshots over {} distinct generations",
        generations.len()
    );
    assert!(generations.len() > 10, "readers ran alongside the writer");
}

/// A query past its deadline is interrupted within 50 ms of the limit.
#[test]
fn a_query_past_its_deadline_times_out_promptly() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("index.cartograph");
    drop(empty_index(&path, 1));
    let store = Store::open_path(&path).unwrap();
    let limit = Duration::from_millis(200);
    let t = Instant::now();
    let snap = store
        .snapshot(Limits {
            deadline: limit,
            ..Limits::default()
        })
        .unwrap();
    let r: Result<i64, QueryError> = snap
        .conn()
        .query_row(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
            [],
            |r| r.get(0),
        )
        .map_err(QueryError::from);
    let took = t.elapsed();
    eprintln!("deadline {limit:?}, interrupted after {took:?}");
    assert!(matches!(r, Err(QueryError::Timeout)), "{r:?}");
    assert!(
        took >= limit && took < limit + Duration::from_millis(50),
        "took {took:?}"
    );
    drop(snap);
    // The connection goes back to the pool usable, with the handler reset.
    let snap = store.snapshot(Limits::default()).unwrap();
    assert_eq!(snap.generation, 1);
}

/// B2: a rebuild writes a new file and renames it into place. The next snapshot reads the new
/// file without a restart; one already open finishes on the old file.
// Windows can't rename over a file SQLite holds open; the indexer's rebuild handles that (IDX-16).
#[cfg(unix)]
#[test]
fn a_rebuilt_file_is_picked_up_without_a_restart() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("index.cartograph");
    let w = empty_index(&path, 1);
    w.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(w);
    let store = Store::open_path(&path).unwrap();
    let old = store.snapshot(Limits::default()).unwrap();
    assert_eq!(old.generation, 1);

    let next = d.path().join("index.cartograph.new");
    drop(empty_index(&next, 7));
    std::fs::rename(&next, &path).unwrap();

    let new = store.snapshot(Limits::default()).unwrap();
    assert_eq!(new.generation, 7);
    assert_eq!(old.generation, 1);
    let still: i64 = old
        .conn()
        .query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        still, 1,
        "an open snapshot keeps reading the file it started on"
    );
    drop((old, new));
    for _ in 0..3 {
        assert_eq!(store.snapshot(Limits::default()).unwrap().generation, 7);
    }
}

#[test]
fn a_missing_or_foreign_workspace_is_not_found_and_names_no_path() {
    let d = tempfile::tempdir().unwrap();
    for ws in ["nope", "../etc", "a/b", "", "x\0y"] {
        let e = Store::open(d.path(), ws).err().expect("no index there");
        assert_eq!(e.code(), ErrorCode::NotFound, "{ws}");
        let body = serde_json::to_string(&e.to_api()).unwrap();
        assert!(!body.contains(&*d.path().to_string_lossy()), "{body}");
    }
    let junk = d.path().join("junk");
    std::fs::write(&junk, vec![0u8; 4096]).unwrap();
    let e = Store::open_path(&junk).err().unwrap();
    assert_eq!(e.code(), ErrorCode::IndexUnavailable);
    assert!(!serde_json::to_string(&e.to_api()).unwrap().contains("junk"));
}

#[test]
fn internal_errors_carry_no_sql() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("index.cartograph");
    drop(empty_index(&path, 1));
    let store = Store::open_path(&path).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let e: QueryError = snap
        .conn()
        .execute("DELETE FROM link", [])
        .map_err(QueryError::from)
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::Internal, "readers are query_only");
    let body = serde_json::to_string(&e.to_api()).unwrap();
    for leak in ["DELETE", "link", "readonly", "SQLITE", "index.cartograph"] {
        assert!(!body.contains(leak), "{leak} in {body}");
    }
}

// ---------------------------------------------------------------- the fixture and SRV-04

struct Built {
    _dir: tempfile::TempDir,
    fixture: Fixture,
    index: PathBuf,
    shas: Vec<String>,
}

/// A small history with adds, edits, a package rename, a delete and a file that comes back.
fn built() -> Built {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    let mut shas = Vec::new();
    let mut commit = |f: &mut Fixture, m: &str| shas.push(f.commit(m));
    f.write("package.json", r#"{"name":"root","private":true}"#);
    f.write(
        "src/a.ts",
        "import { b } from './b';\nimport l from 'lib';\n",
    );
    commit(&mut f, "add a");
    f.write("src/b/index.ts", "export const b = 1;\n");
    f.write(
        "lib/package.json",
        r#"{"name":"lib","exports":"./src/index.ts"}"#,
    );
    f.write("lib/src/index.ts", "export default 1;\n");
    commit(&mut f, "add b and lib");
    f.write("src/b.ts", "export const b = 2;\n");
    commit(&mut f, "a better candidate");
    f.write(
        "lib/package.json",
        r#"{"name":"other","main":"dist/index.js"}"#,
    );
    f.write(
        "src/c.ts",
        "import a from './a.js';\nimport o from 'other';\nimport b from './b';\n",
    );
    commit(&mut f, "rename the package");
    f.remove("src/b.ts");
    commit(&mut f, "delete b");
    f.write(
        "src/d/index.ts",
        "import c from '../c';\nimport a from '../a';\n",
    );
    f.write("web/index.ts", "import c from '../src/c';\n");
    commit(&mut f, "add d and web");
    f.write("src/b.ts", "import d from './d';\n");
    commit(&mut f, "b comes back");
    f.remove("src/a.ts");
    commit(&mut f, "delete a");
    let index = d.path().join("data/workspaces/w-00000000/index.cartograph");
    std::fs::create_dir_all(index.parent().unwrap()).unwrap();
    let mut cache = FactCache::open(&d.path().join("data/cache/facts.db")).unwrap();
    cartograph_index::build_single(&f.dir, None, &index, &mut cache, 2).unwrap();
    Built {
        _dir: d,
        fixture: f,
        index,
        shas,
    }
}

fn at(s: &str) -> GraphQuery {
    GraphQuery::from_params(&GraphParams {
        at: Some(s.into()),
        ..Default::default()
    })
    .unwrap()
}

/// SRV-04's done-when: at every commit of the fixture (all 8, under the research note's 24) the
/// slice equals a brute-force reference: the tree read from git, and the links a from-scratch
/// index of that tree finds.
#[test]
fn graph_equals_brute_force_at_every_commit() {
    let b = built();
    let store = Store::open_path(&b.index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let repo = Repo::open(&b.fixture.dir).unwrap();
    let mut cache = FactCache::open(&b._dir.path().join("data/cache/facts.db")).unwrap();
    let (steps, facts, ..) =
        cartograph_index::build::walk_and_extract(&repo, repo.head().unwrap(), &mut cache, 2)
            .unwrap();
    assert_eq!(steps.len(), b.shas.len());
    for (seq, (step, sha)) in steps.iter().zip(&b.shas).enumerate() {
        let slice = snap.graph(&at(sha)).unwrap();
        let got = slice.at.as_ref().unwrap();
        assert_eq!((got.seq, got.sha.as_str()), (seq as u64, sha.as_str()));

        let tree = repo.tree_at(step.sha, cartograph_parse::tracked).unwrap();
        let files: BTreeSet<&String> = tree.keys().collect();
        assert_eq!(
            slice.nodes.name.iter().collect::<BTreeSet<_>>(),
            files,
            "files at {seq}"
        );
        assert_eq!(
            slice.nodes.name.len(),
            files.len(),
            "no file twice at {seq}"
        );

        let direct = tree
            .iter()
            .map(|(p, o)| (p.clone(), facts[&(*o, extractor_for(p).unwrap().1)].clone()))
            .collect();
        let want: Vec<(String, String)> = from_scratch(&direct)
            .into_iter()
            .map(|(_, a, b)| (a, b))
            .collect();
        let mut have: Vec<(String, String)> = (0..slice.links.id.len())
            .map(|i| {
                let name = |n: u32| slice.nodes.name[n as usize].clone();
                (name(slice.links.from[i]), name(slice.links.to[i]))
            })
            .collect();
        have.sort();
        let mut want = want;
        want.sort();
        assert_eq!(have, want, "links at {seq}");
        for i in 0..slice.links.id.len() {
            let (from, to) = (slice.links.added_seq[i], slice.links.removed_seq[i]);
            assert!(
                from <= seq as u64 && (to == -1 || to > seq as i64),
                "interval at {seq}"
            );
        }
    }
}

/// FE-08 and IDX-25's gate: the same index read twice places every node alike, the positions are
/// the `layout` rows live at the step, and a rolled-up link counts the file links it stands for.
#[test]
fn graph_levels_read_the_stored_layout_and_count_member_links() {
    use cartograph_api::types::{GraphSlice, Lod};
    use std::collections::BTreeMap;
    let b = built();
    let q = |sha: &str, lod: Lod| GraphQuery { lod, ..at(sha) };
    let read = |sha: &str, lod: Lod| -> std::sync::Arc<GraphSlice> {
        // A store of its own each time: nothing is shared but the file.
        let store = Store::open_path(&b.index).unwrap();
        let snap = store.snapshot(Limits::default()).unwrap();
        snap.graph(&q(sha, lod)).unwrap()
    };
    let conn = Connection::open(&b.index).unwrap();
    let layout_at = |node: &str, seq: u64| -> Option<(f32, f32, f32)> {
        let id: i64 = node.trim_start_matches('n').parse().unwrap();
        conn.query_row(
            "SELECT x, y, r FROM layout WHERE node = ?1 AND from_seq <= ?2
               AND (to_seq IS NULL OR to_seq > ?2)",
            params![id, seq as i64],
            |r| {
                Ok((
                    r.get::<_, f64>(0)? as f32,
                    r.get::<_, f64>(1)? as f32,
                    r.get::<_, f64>(2)? as f32,
                ))
            },
        )
        .ok()
    };
    // Each file node's (module or service, service), from the node table.
    let holders: BTreeMap<String, (String, String)> = conn
        .prepare(
            "SELECT id, coalesce(parent, service), service FROM node
             WHERE kind = 'file' AND service IS NOT NULL",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                format!("n{}", r.get::<_, i64>(0)?),
                (
                    format!("n{}", r.get::<_, i64>(1)?),
                    format!("n{}", r.get::<_, i64>(2)?),
                ),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut placed = 0;
    let mut rolled = 0;
    for sha in &b.shas {
        for lod in [Lod::File, Lod::Module, Lod::Service] {
            let (one, two) = (read(sha, lod), read(sha, lod));
            assert_eq!(one.lod, lod);
            assert_eq!(one.nodes.id, two.nodes.id);
            assert_eq!(one.nodes.x, two.nodes.x, "same x twice at {sha} {lod:?}");
            assert_eq!(one.nodes.y, two.nodes.y, "same y twice at {sha} {lod:?}");
            assert_eq!(one.nodes.size, two.nodes.size);
            let seq = one.at.as_ref().unwrap().seq;
            for i in 0..one.nodes.id.len() {
                let got = (one.nodes.x[i], one.nodes.y[i], one.nodes.size[i]);
                match layout_at(&one.nodes.id[i].0, seq) {
                    Some(want) => {
                        assert_eq!(got, want, "{} at {seq}", one.nodes.name[i]);
                        placed += 1;
                    }
                    None => assert_eq!(got, (0.0, 0.0, 1.0)),
                }
            }
        }
        // Rolled-up counts: every file link between two holders, once, under its pair.
        let files = read(sha, Lod::File);
        for (lod, pick) in [
            (
                Lod::Service,
                (|h: &(String, String)| h.1.clone()) as fn(&_) -> String,
            ),
            (Lod::Module, |h: &(String, String)| h.0.clone()),
        ] {
            let mut want: BTreeMap<(String, String), u32> = BTreeMap::new();
            for j in 0..files.links.id.len() {
                let end = |n: u32| holders.get(&files.nodes.id[n as usize].0).map(pick);
                if let (Some(a), Some(b)) = (end(files.links.from[j]), end(files.links.to[j]))
                    && a != b
                {
                    *want.entry((a, b)).or_default() += files.links.weight[j];
                }
            }
            let s = read(sha, lod);
            let have: BTreeMap<(String, String), u32> = (0..s.links.id.len())
                .map(|j| {
                    let id = |n: u32| s.nodes.id[n as usize].0.clone();
                    ((id(s.links.from[j]), id(s.links.to[j])), s.links.weight[j])
                })
                .collect();
            assert_eq!(have.len(), s.links.id.len(), "one link per pair");
            assert_eq!(have, want, "{lod:?} link counts at {sha}");
            rolled += have.len();
            // A parent index points at a node of the slice.
            for p in &s.nodes.parent {
                assert!(*p >= -1 && (*p as i64) < s.nodes.id.len() as i64);
            }
        }
    }
    assert!(placed > 0, "the fixture has stored positions");
    assert!(rolled > 0, "the fixture has rolled-up links");
}

#[test]
fn at_resolves_dates_short_shas_and_now() {
    let b = built();
    let store = Store::open_path(&b.index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    assert_eq!(snap.generation, 1);
    let last = b.shas.len() as u64 - 1;
    assert_eq!(snap.resolve_at(&At::Now).unwrap().seq, last);
    // The fixture's clock starts at 1,600,000,000 and adds 60 s a commit.
    let t = |k: i64| At::Time(1_600_000_000 + 60 * k);
    assert_eq!(snap.resolve_at(&t(3)).unwrap().seq, 2);
    assert_eq!(snap.resolve_at(&t(3)).unwrap().sha, b.shas[2]);
    assert_eq!(
        snap.resolve_at(&At::Time(1_600_000_000 + 60 * 3 + 59))
            .unwrap()
            .seq,
        2
    );
    assert_eq!(
        snap.resolve_at(&t(0)).unwrap_err().code(),
        ErrorCode::NotFound
    );
    let short = At::parse(&b.shas[4][..10]).unwrap();
    assert_eq!(snap.resolve_at(&short).unwrap().seq, 4);
    assert_eq!(
        snap.resolve_at(&At::parse(&"0".repeat(40)).unwrap())
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    let date = rfc3339(1_600_000_000 + 60 * 5);
    assert_eq!(snap.graph(&at(&date)).unwrap().at.as_ref().unwrap().seq, 4);
}

#[test]
fn node_references_resolve_or_list_candidates() {
    let b = built();
    let store = Store::open_path(&b.index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let head = b.shas.len() as u64 - 1;
    let id = snap.resolve_node("src/c.ts", head).unwrap();
    assert_eq!(snap.resolve_node(&id, head).unwrap(), id);
    assert_eq!(snap.resolve_node("c.ts", head).unwrap(), id);
    assert_eq!(snap.resolve_node("repo:src/c.ts", head).unwrap(), id);
    assert_eq!(
        snap.resolve_node("other:src/c.ts", head)
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    // a.ts was deleted at head but lives at step 0.
    assert_eq!(
        snap.resolve_node("a.ts", head).unwrap_err().code(),
        ErrorCode::NotFound
    );
    assert!(snap.resolve_node("a.ts", 0).is_ok());
    let e = snap.resolve_node("index.ts", head).unwrap_err();
    assert_eq!(e.code(), ErrorCode::Ambiguous);
    let api = e.to_api();
    let c = api.details.unwrap()["candidates"].as_array().unwrap().len();
    assert_eq!(c, 4, "lib/src, src/b, src/d and web");
    for bad in ["", "a\u{7}b", &"x".repeat(1025)] {
        assert_eq!(
            snap.resolve_node(bad, head).unwrap_err().code(),
            ErrorCode::BadRequest
        );
    }
    assert_eq!(
        snap.resolve_node("x' OR 1=1 --", head).unwrap_err().code(),
        ErrorCode::NotFound
    );
}

#[test]
fn graph_filters_and_refusals() {
    let b = built();
    let store = Store::open_path(&b.index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let all = snap.graph(&at("now")).unwrap();
    assert!(!all.links.id.is_empty());
    let none = GraphQuery {
        kinds: Some(vec![LinkKind::Http]),
        ..at("now")
    };
    assert!(snap.graph(&none).unwrap().links.id.is_empty());
    assert_eq!(snap.graph(&none).unwrap().nodes.id, all.nodes.id);
    let other = GraphQuery {
        repo: Some(RepoId("nope".into()).0),
        ..at("now")
    };
    assert_eq!(snap.graph(&other).unwrap_err().code(), ErrorCode::NotFound);

    let refused = |p: GraphParams| GraphQuery::from_params(&p).unwrap_err().code();
    use cartograph_api::types::Lod;
    assert_eq!(
        refused(GraphParams {
            lod: Some(Lod::Workspace),
            ..Default::default()
        }),
        ErrorCode::BadRequest
    );
    assert_eq!(
        refused(GraphParams {
            range: Some("a..b".into()),
            ..Default::default()
        }),
        ErrorCode::BadRequest
    );
    assert_eq!(
        refused(GraphParams {
            kinds: Some("imports,teleport".into()),
            ..Default::default()
        }),
        ErrorCode::BadRequest
    );
    assert_eq!(
        refused(GraphParams {
            at: Some("yesterday".into()),
            ..Default::default()
        }),
        ErrorCode::BadRequest
    );
    drop(snap);

    let tight = store
        .snapshot(Limits {
            max_rows: 3,
            ..Limits::default()
        })
        .unwrap();
    let e = tight.graph(&at("now")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::TooLarge);
    assert!(e.to_api().action.contains("kinds="));
}

#[test]
fn workspace_repos_and_freshness_read_stored_state() {
    let b = built();
    let data = b
        .index
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let store = Store::open(data, "w-00000000").unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let w = snap.workspace_summary("w-00000000").unwrap();
    assert_eq!((w.name.as_str(), w.repos, w.commits), ("repo", 1, 8));
    assert_eq!(
        w.files as usize,
        snap.graph(&at("now")).unwrap().nodes.id.len()
    );
    let repos = snap.repos().unwrap();
    assert_eq!(
        repos[0].indexed_commit.as_deref(),
        b.shas.last().map(String::as_str)
    );
    let f = snap.freshness().unwrap();
    assert_eq!(f.state, cartograph_api::types::IndexState::Current);
    assert_eq!(
        i64::from(f.format_version),
        cartograph_store::FORMAT_VERSION
    );
    assert!(f.built_at.is_some());
}

/// The graph slice on a real index, timed, and checked at 24 sampled steps against the indexer's
/// own as-of reader. `CARTOGRAPH_INDEX=<index.cartograph> cargo test --release -p cartograph-query
/// -- --ignored --nocapture real_index`
#[test]
#[ignore = "needs CARTOGRAPH_INDEX"]
fn real_index_slices_match_and_are_timed() {
    let path = PathBuf::from(std::env::var_os("CARTOGRAPH_INDEX").expect("CARTOGRAPH_INDEX"));
    let t = Instant::now();
    let store = Store::open_path(&path).unwrap();
    eprintln!("open (format gate and quick_check): {:?}", t.elapsed());
    let snap = store.snapshot(Limits::default()).unwrap();
    let head = snap.resolve_at(&At::Now).unwrap().seq;
    let mut times = Vec::new();
    for k in 0..24u64 {
        let seq = head * k / 23;
        let sha: Vec<u8> = snap
            .conn()
            .query_row(
                "SELECT sha FROM commits WHERE seq = ?1",
                [seq as i64],
                |r| r.get(0),
            )
            .unwrap();
        let q = at(&crate::resolve::hex(&sha));
        let t = Instant::now();
        let slice = snap.graph(&q).unwrap();
        times.push(t.elapsed());
        let mut have: Vec<(String, String)> = (0..slice.links.id.len())
            .map(|i| {
                let n = |x: u32| slice.nodes.name[x as usize].clone();
                (n(slice.links.from[i]), n(slice.links.to[i]))
            })
            .collect();
        have.sort();
        let mut want: Vec<(String, String)> = cartograph_index::build::links_at(snap.conn(), seq)
            .unwrap()
            .into_iter()
            .map(|(_, a, b)| (a, b))
            .collect();
        want.sort();
        assert_eq!(have, want, "step {seq}");
        if k == 23 {
            eprintln!(
                "head: {} files, {} links",
                slice.nodes.id.len(),
                slice.links.id.len()
            );
        }
    }
    times.sort();
    eprintln!("24 slices: median {:?}, max {:?}", times[12], times[23]);
}

/// Two repos synced onto one timeline, as a workspace is: paths are stored `api/…`, `web/…`.
#[test]
fn a_two_repo_workspace_filters_counts_and_names_by_repo() {
    let d = tempfile::tempdir().unwrap();
    let mut api = Fixture::init(&d.path().join("api"));
    api.write("src/a.ts", "export const a = 1;\n");
    api.write("src/b.ts", "import { a } from './a';\n");
    api.commit("api one");
    api.write("src/c.ts", "import { a } from './a';\n");
    api.commit("api two");
    let mut web = Fixture::init(&d.path().join("web"));
    web.write("src/w.ts", "export const w = 1;\n");
    web.commit("web one");
    let dir = d.path().join("data/workspaces/0000000a");
    let spec = |f: &Fixture| cartograph_index::timeline::RepoSpec {
        name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
        path: f.dir.clone(),
        ..Default::default()
    };
    let ws = |repos| cartograph_index::sync::Workspace {
        dir: dir.clone(),
        repos,
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let sync = |w: &cartograph_index::sync::Workspace, cache: &mut FactCache| {
        cartograph_index::sync::sync(w, &Default::default(), cache).unwrap();
    };
    sync(&ws(vec![spec(&api), spec(&web)]), &mut cache);
    let index = dir.join("index.cartograph");
    {
        let store = Store::open_path(&index).unwrap();
        let snap = store.snapshot(Limits::default()).unwrap();
        // repo= keeps that repo's files and the links between them.
        let only_api = GraphQuery {
            repo: Some("api".into()),
            ..at("now")
        };
        let slice = snap.graph(&only_api).unwrap();
        assert_eq!(
            slice.nodes.name,
            ["api/src/a.ts", "api/src/b.ts", "api/src/c.ts"]
        );
        assert_eq!(slice.links.id.len(), 2);
        // Each repo counts its own commits, not the workspace's.
        let counts: Vec<(String, u32)> = snap
            .repos()
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.commits))
            .collect();
        assert_eq!(counts, [("api".into(), 2), ("web".into(), 1)]);
        // repo:path names the path inside the repo, though the index stores it with the prefix.
        let head = snap.resolve_at(&crate::At::Now).unwrap().seq;
        let id = snap.resolve_node("api/src/b.ts", head).unwrap();
        assert_eq!(snap.resolve_node("api:src/b.ts", head).unwrap(), id);
    }
    // web leaves the workspace: no longer listed or counted, but its history still answers repo=.
    sync(&ws(vec![spec(&api)]), &mut cache);
    let store = Store::open_path(&index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let names: Vec<String> = snap.repos().unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["api"]);
    assert_eq!(snap.workspace_summary("0000000a").unwrap().repos, 1);
    let web_then = GraphQuery {
        repo: Some("web".into()),
        ..at("now")
    };
    assert!(
        snap.graph(&web_then).is_ok(),
        "a removed repo is still a known name"
    );
}

/// IDX-20 to IDX-22's reads: coverage from the last run's totals, the open unresolved imports as of
/// a commit, runs newest first, extractors with their live files, and services with their rule.
#[test]
fn coverage_runs_extractors_and_services_read_what_the_index_stored() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("index.cartograph");
    let w = empty_index(&path, 1);
    w.execute_batch(
        "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
           VALUES (1, 1, x'aa01', 100, 100, 'a', 'one'), (2, 1, x'aa02', 200, 200, 'a', 'two');
         INSERT INTO file (id, repo) VALUES (1, 1), (2, 1);
         INSERT INTO file_path (file, path, from_seq, to_seq) VALUES
           (1, 'r/src/a.ts', 1, NULL), (2, 'r/src/b.ts', 1, NULL);
         INSERT INTO file_version (file, blob, from_seq, to_seq) VALUES
           (1, x'b1', 1, NULL), (2, x'b2', 1, 2), (2, x'b3', 2, NULL);
         INSERT INTO extractor (id, name, version) VALUES
           (1, 'ts-imports', '0.9.0'), (2, 'ts-imports/ts', '1.0.0'), (3, 'go-imports/go', '1.0.0');
         INSERT INTO fact (blob, extractor, payload) VALUES
           (x'b1', 2, x''), (x'b2', 2, x''), (x'b3', 1, x'');
         INSERT INTO node (id, kind, file, key, name, detected_by) VALUES
           (10, 'service', NULL, 'service:r:', 'r', 'repo'),
           (11, 'library', NULL, 'service:r:packages/ui', '@r/ui', 'pnpm'),
           (12, 'service', NULL, 'service:r:old', 'old', 'repo'),
           (20, 'file', 1, NULL, NULL, NULL);
         INSERT INTO node_span (node, from_seq, to_seq) VALUES (10, 1, NULL), (11, 1, NULL), (12, 1, 2);
         INSERT INTO unresolved (file, spec, line, stage, reason, from_seq, to_seq) VALUES
           (1, '@shared/money', 3, 'resolve', 'not-found', 1, NULL),
           (1, './gone', 9, 'resolve', 'not-found', 1, 2);
         INSERT INTO coverage_row (row, seq, done, total, unresolved) VALUES
           ('files', 2, 2, 2, 0), ('imports', 2, 4, 5, 1), ('calls', 2, 0, 0, 0),
           ('handlers', 2, 0, 0, 0), ('routes', 2, 0, 0, 0), ('consumers', 2, 0, 0, 0);
         INSERT INTO run (id, started_at, finished_at, writer_version, files_total, files_parsed,
                          imports_total, imports_resolved) VALUES
           (1, 1000, 1041, '0.1.0', 2, 2, 5, 3),
           (2, 2000, NULL, '0.2.0', NULL, NULL, NULL, NULL);
         PRAGMA wal_checkpoint(TRUNCATE);",
    )
    .unwrap();
    drop(w);
    let store = Store::open_path(&path).unwrap();
    let s = store.snapshot(Limits::default()).unwrap();

    // The last run's figures are the derived row; any other date counts from the intervals.
    let rows = s.coverage(&At::parse("now").unwrap()).unwrap();
    let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "files",
            "imports",
            "calls",
            "handlers",
            "routes",
            "consumers"
        ]
    );
    assert_eq!(
        (rows[0].done, rows[0].total, rows[0].target),
        (2, 2, Some(98.0))
    );
    assert!(rows[0].gap.is_none());
    assert_eq!((rows[1].done, rows[1].total), (4, 5));
    assert_eq!(rows[1].gap.as_ref().unwrap().what, "1 import");
    assert_eq!(rows[1].gaps[0].reason, "not-found");
    assert_eq!(rows[3].target, None);
    let then = s.coverage(&At::parse("aa01").unwrap()).unwrap();
    assert_eq!(
        (then[0].done, then[0].total, then[1].done, then[1].total),
        (2, 2, 0, 2),
        "a past date: paths alive, import links alive plus the gaps open then"
    );

    let now = s
        .unresolved("imports", &At::parse("now").unwrap(), Page::default())
        .unwrap();
    assert_eq!(now.rows.len(), 1);
    let u = &now.rows[0];
    assert_eq!(
        (u.file.0.as_str(), u.path.as_str(), u.line, u.text.as_str()),
        (node_id(20).as_str(), "r/src/a.ts", 3, "@shared/money")
    );
    let then = s
        .unresolved("imports", &At::parse("aa01").unwrap(), Page::default())
        .unwrap();
    assert_eq!(
        then.rows.len(),
        2,
        "a closed interval is listed as of a commit inside it"
    );
    assert!(matches!(
        s.unresolved("nope", &At::parse("now").unwrap(), Page::default()),
        Err(QueryError::NotFound { .. })
    ));

    let runs = s.runs(Page::default()).unwrap().rows;
    assert_eq!(runs.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(runs[1].files_parsed, Some(2));
    assert!(runs[0].finished_at.is_none() && runs[0].files_total.is_none());

    let ex = s.extractors().unwrap();
    let ex: Vec<_> = ex
        .iter()
        .map(|e| (e.name.as_str(), e.version.as_str(), e.files))
        .collect();
    assert_eq!(ex, [("go-imports", "1.0.0", 0), ("ts-imports", "1.0.0", 2)]);

    let svc = s.services().unwrap();
    let svc: Vec<_> = svc
        .iter()
        .map(|x| {
            (
                x.name.as_str(),
                x.repo.0.as_str(),
                x.paths.clone(),
                x.library,
                x.detected_by.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        svc,
        [
            ("r", "r", vec![], false, Some("repo")),
            (
                "@r/ui",
                "r",
                vec!["packages/ui".to_owned()],
                true,
                Some("pnpm")
            ),
        ]
    );
}

/// SRV-04: the module and service slices roll up from the generation's in-memory graph. At every
/// step, with and without repo=, they equal the roll-up read from the tables: the same nodes in
/// the same order, the same links, ids, weights and added_seq.
#[test]
fn in_memory_roll_ups_equal_the_sql_ones_at_every_step_and_repo() {
    use cartograph_api::types::Lod;
    let d = tempfile::tempdir().unwrap();
    let mut api = Fixture::init(&d.path().join("api"));
    api.write(
        "package.json",
        r#"{"name":"api","private":true,"workspaces":["lib"]}"#,
    );
    api.write("src/a.ts", "export const a = 1;\n");
    api.write("src/m/b.ts", "import { a } from '../a';\n");
    api.write(
        "lib/package.json",
        r#"{"name":"lib","main":"src/index.ts"}"#,
    );
    api.write("lib/src/index.ts", "import { a } from '../../src/a';\n");
    api.commit("api one");
    let mut web = Fixture::init(&d.path().join("web"));
    web.write("package.json", r#"{"name":"web","private":true}"#);
    web.write("src/w.ts", "export const w = 1;\n");
    web.write("src/v/x.ts", "import { w } from '../w';\n");
    web.commit("web one");
    api.write(
        "src/m/c.ts",
        "import { a } from '../a';\nimport l from 'lib';\n",
    );
    api.write(
        "src/n/d.ts",
        "import { c } from '../m/c';\nimport { b } from '../m/b';\n",
    );
    api.commit("api two");
    api.remove("src/m/b.ts");
    api.write("lib/src/e.ts", "import { c } from '../../src/m/c';\n");
    api.commit("api three");
    web.write(
        "src/v/y.ts",
        "import { x } from './x';\nimport { w } from '../w';\n",
    );
    web.commit("web two");
    let dir = d.path().join("data/workspaces/0000000b");
    let spec = |f: &Fixture| cartograph_index::timeline::RepoSpec {
        name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
        path: f.dir.clone(),
        ..Default::default()
    };
    let w = cartograph_index::sync::Workspace {
        dir: dir.clone(),
        repos: vec![spec(&api), spec(&web)],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    cartograph_index::sync::sync(&w, &Default::default(), &mut cache).unwrap();
    let store = Store::open_path(&dir.join("index.cartograph")).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    // Every step by its sha: all but the last are past ones.
    let shas: Vec<String> = snap
        .conn()
        .prepare("SELECT lower(hex(sha)) FROM commits ORDER BY seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(shas.len() >= 5, "{} steps", shas.len());
    let (mut modules, mut links) = (0, 0);
    for (seq, sha) in shas.iter().enumerate() {
        for repo in [None, Some("api"), Some("web")] {
            for lod in [Lod::Module, Lod::Service] {
                let q = GraphQuery {
                    lod,
                    repo: repo.map(Into::into),
                    ..at(sha)
                };
                let mem = snap.graph(&q).unwrap();
                let sql = snap.rolled_up_sql(&q).unwrap();
                assert_eq!(
                    serde_json::to_value(&*mem).unwrap(),
                    serde_json::to_value(&sql).unwrap(),
                    "{lod:?} at {seq}, repo {repo:?}"
                );
                modules += mem
                    .nodes
                    .kind
                    .iter()
                    .filter(|k| **k == cartograph_api::types::NodeKind::Module)
                    .count();
                links += mem.links.id.len();
            }
        }
    }
    assert!(modules > 0, "the fixture has modules");
    assert!(links > 0, "the fixture has rolled-up links");
}

/// XS-23: owners come from the repository's CODEOWNERS at each commit; a change to it changes who
/// owns a file from that commit and not before, and `…/owners` counts what is owned now.
#[test]
fn owners_follow_codeowners_at_each_commit() {
    let d = tempfile::tempdir().unwrap();
    let mut api = Fixture::init(&d.path().join("api"));
    api.write("src/a.ts", "export const a = 1;\n");
    api.write("lib/b.ts", "export const b = 1;\n");
    api.write("CODEOWNERS", "* @everyone\n");
    let c1 = api.commit("api one");
    api.write(
        "CODEOWNERS",
        "* @everyone\n/src/ @acme/core\nb.ts docs@example.com\n",
    );
    let c2 = api.commit("api two: new owners");
    let mut web = Fixture::init(&d.path().join("web"));
    web.write("src/w.ts", "export const w = 1;\n");
    web.commit("web one");
    let dir = d.path().join("data/workspaces/0000000b");
    let spec = |f: &Fixture| cartograph_index::timeline::RepoSpec {
        name: f.dir.file_name().unwrap().to_string_lossy().into_owned(),
        path: f.dir.clone(),
        ..Default::default()
    };
    let w = cartograph_index::sync::Workspace {
        dir: dir.clone(),
        repos: vec![spec(&api), spec(&web)],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    cartograph_index::sync::sync(&w, &Default::default(), &mut cache).unwrap();
    let store = Store::open_path(&dir.join("index.cartograph")).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let now = snap.resolve_at(&crate::At::Now).unwrap().seq;
    let owners_at = |path: &str, sha: &str| -> Vec<String> {
        let id = snap.resolve_node(path, now).unwrap();
        snap.node_detail(&id.to_string(), &crate::At::Sha(sha.to_owned()))
            .unwrap()
            .owners
    };
    assert_eq!(owners_at("api/src/a.ts", &c1), ["@everyone"]);
    assert_eq!(owners_at("api/src/a.ts", &c2), ["@acme/core"]);
    assert_eq!(owners_at("api/lib/b.ts", &c1), ["@everyone"]);
    assert_eq!(owners_at("api/lib/b.ts", &c2), ["docs@example.com"]);
    // web has no CODEOWNERS: nobody owns its files.
    assert!(owners_at("web/src/w.ts", &c2).is_empty());
    let all = snap.owners().unwrap();
    let got: Vec<(&str, u32)> = all.iter().map(|o| (o.id.as_str(), o.nodes)).collect();
    assert_eq!(got, [("@acme/core", 1), ("docs@example.com", 1)]);
    assert_eq!(all[0].patterns, ["/src/"]);
}

/// XS-02's done-when: a pnpm workspace with two Dockerfiles and a compose file yields its services,
/// each with its rule and the file the rule read, and `http://orders:8080` resolves to `orders`;
/// the aliases have intervals, so a rename in the compose file closes one and opens the other.
#[test]
fn a_compose_host_resolves_to_its_service_at_every_step() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("shop"));
    f.write("package.json", r#"{"name":"shop","private":true}"#);
    f.write("pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    f.write("packages/orders/package.json", r#"{"name":"orders"}"#);
    f.write("packages/orders/Dockerfile", "FROM node:22\n");
    f.write("packages/orders/src/index.ts", "export const o = 1;\n");
    f.write("packages/web/package.json", r#"{"name":"web"}"#);
    f.write("packages/web/Dockerfile", "FROM node:22\n");
    f.write("packages/web/src/index.ts", "export const w = 1;\n");
    f.write("packages/money/package.json", r#"{"name":"money"}"#);
    f.write("packages/money/src/index.ts", "export const m = 1;\n");
    let compose = |orders: &str| {
        format!(
            "services:
  {orders}:
    build: ./packages/orders
    expose: ['8080']
    networks:
      default:
        aliases: [orders-api]
  web:
    build:
      context: ./packages/web
    ports: ['3000:3000']
"
        )
    };
    f.write("docker-compose.yml", &compose("orders"));
    let first = f.commit("compose");
    f.write("docker-compose.yml", &compose("sales"));
    let second = f.commit("rename the compose service");

    let index = d.path().join("data/index.cartograph");
    std::fs::create_dir_all(index.parent().unwrap()).unwrap();
    let mut cache = FactCache::open(&d.path().join("data/facts.db")).unwrap();
    cartograph_index::build_single(&f.dir, None, &index, &mut cache, 2).unwrap();
    let store = Store::open_path(&index).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    let svc: Vec<_> = snap
        .services()
        .unwrap()
        .into_iter()
        .map(|s| {
            (
                s.name,
                s.library,
                s.detected_by.unwrap(),
                s.detect_source.unwrap(),
            )
        })
        .collect();
    let row = |n: &str, lib: bool| {
        (
            n.to_owned(),
            lib,
            "pnpm".to_owned(),
            "pnpm-workspace.yaml".to_owned(),
        )
    };
    assert_eq!(
        svc,
        [row("money", true), row("orders", false), row("web", false)]
    );

    let now = At::Now;
    let hit = snap.resolve_host("http://orders:8080/orders/1?x=1", &At::Sha(first.clone()));
    let hit = hit.unwrap().expect("orders resolves at the first commit");
    assert_eq!((hit.name.as_str(), hit.host.as_str()), ("orders", "orders"));
    assert_eq!(
        (hit.port, hit.source.as_str()),
        (Some(8080), "docker-compose.yml")
    );
    // The network alias, case-folded; a port the service doesn't declare; no port at all.
    let name = |t: &str, at: &At| snap.resolve_host(t, at).unwrap().map(|m| m.name);
    assert_eq!(
        name("ORDERS-API:8080", &At::Sha(first.clone())).as_deref(),
        Some("orders")
    );
    assert_eq!(name("http://orders:9999", &At::Sha(first.clone())), None);
    assert_eq!(
        name("orders", &At::Sha(first.clone())).as_deref(),
        Some("orders")
    );
    assert_eq!(
        name("web:3000", &At::Sha(first.clone())).as_deref(),
        Some("web")
    );
    // The rename closed `orders` and opened `sales` at the second commit; the network alias stays.
    assert_eq!(name("orders:8080", &At::Sha(second.clone())), None);
    assert_eq!(
        name("sales:8080", &At::Sha(second.clone())).as_deref(),
        Some("orders")
    );
    assert_eq!(name("sales:8080", &At::Sha(first)), None);
    assert_eq!(name("orders-api:8080", &now).as_deref(), Some("orders"));
    assert_eq!(name("[::1]:8080", &now), None);
    assert_eq!(name("nobody:1", &now), None);
}
