//! HD-06 properties on generated git histories, through real git, the extractors and the index file.
//!
//! Case counts: 256 by default (proptest's own default, so `PROPTEST_CASES` overrides it; the
//! nightly run sets 10000). A failing case's seed is written to
//! `fixtures/gen/tests/properties.proptest-regressions`, which is committed and replayed first on
//! every later run.
//!
//! The properties that need the sync command (incremental equals full, SIGKILL) run the binary and
//! live in `crates/cartograph-cli/tests/props.rs`.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use cartograph_facts::{FactCache, from_scratch};
use cartograph_git::Repo;
use cartograph_index::build::{links_at, walk_and_extract};
use cartograph_parse::extractor_for;
use cartograph_store::Index;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use support::{Commit, import};

/// Every path a generated history may touch: sources that import each other, a folder index, a
/// local package and the root manifest.
const SOURCES: &[&str] = &[
    "src/a.ts",
    "src/b.ts",
    "src/c.tsx",
    "src/d/index.ts",
    "src/e.js",
    "lib/src/index.ts",
];
const SPECS: &[&str] = &[
    "./a",
    "./b",
    "./c",
    "./d",
    "./e.js",
    "lib",
    "other",
    "../lib/src/index",
    "./missing",
];
const LIB: &[&str] = &[
    r#"{"name":"lib","exports":"./src/index.ts"}"#,
    r#"{"name":"lib","version":"2.0.0","exports":"./src/index.ts"}"#,
    r#"{"name":"other","main":"src/index.ts"}"#,
];

#[derive(Debug, Clone)]
enum Op {
    /// Rewrite source `f` with the imports in `mask`.
    Source(usize, u16),
    /// Rewrite `lib/package.json`.
    Manifest(usize),
    Delete(usize),
    /// Move source `f` to source `g`'s path, as a rename.
    Move(usize, usize),
}

fn op() -> impl Strategy<Value = Op> {
    let f = 0..SOURCES.len();
    prop_oneof![
        5 => (f.clone(), any::<u16>()).prop_map(|(f, m)| Op::Source(f, m)),
        2 => (0..LIB.len()).prop_map(Op::Manifest),
        2 => (0..SOURCES.len() + 1).prop_map(Op::Delete),
        1 => (f.clone(), f).prop_map(|(f, g)| Op::Move(f, g)),
    ]
}

/// The commits a list of op lists makes, the first one also writing the root manifest.
fn history(steps: &[Vec<Op>]) -> Vec<Commit> {
    let mut tree: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for (i, ops) in steps.iter().enumerate() {
        let mut c = Commit::new();
        if i == 0 {
            c.insert(
                "package.json".into(),
                Some(r#"{"name":"root","private":true}"#.into()),
            );
        }
        for o in ops {
            match *o {
                Op::Source(f, mask) => {
                    let body: String = SPECS
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| mask & (1 << j) != 0)
                        .map(|(j, s)| format!("import x{j} from '{s}';\n"))
                        .chain([format!("export const r = {mask};\n")])
                        .collect();
                    c.insert(SOURCES[f].into(), Some(body));
                }
                Op::Manifest(v) => {
                    c.insert("lib/package.json".into(), Some(LIB[v].into()));
                }
                Op::Delete(f) => {
                    let p = SOURCES.get(f).copied().unwrap_or("lib/package.json");
                    c.insert(p.into(), None);
                }
                Op::Move(f, g) if f != g => {
                    let body = c
                        .get(SOURCES[f])
                        .cloned()
                        .flatten()
                        .or_else(|| tree.get(SOURCES[f]).cloned());
                    if let Some(b) = body {
                        c.insert(SOURCES[f].into(), None);
                        c.insert(SOURCES[g].into(), Some(b));
                    }
                }
                Op::Move(..) => {}
            }
        }
        for (p, b) in &c {
            match b {
                Some(b) => tree.insert(p.clone(), b.clone()),
                None => tree.remove(p),
            };
        }
        out.push(c);
    }
    out
}

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/properties.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    /// Replay equals direct indexing: at every commit of a random history, the links the index's
    /// intervals give equal the links computed from nothing over that commit's tree.
    #[test]
    fn replay_equals_direct_indexing(steps in prop::collection::vec(prop::collection::vec(op(), 1..4), 1..10)) {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("repo");
        import(&dir, &history(&steps));
        let out = d.path().join("index.cartograph");
        let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
        cartograph_index::build_single(&dir, None, &out, &mut cache, 1).unwrap();
        let ix = Index::open_read(&out).unwrap();

        let repo = Repo::open(&dir).unwrap();
        let (walked, facts, ..) = walk_and_extract(&repo, repo.head().unwrap(), &mut cache, 1).unwrap();
        prop_assert_eq!(walked.len(), steps.len());
        for (seq, step) in walked.iter().enumerate() {
            let tree = repo.tree_at(step.sha, cartograph_parse::tracked).unwrap();
            let direct = tree
                .iter()
                .map(|(p, o)| (p.clone(), facts[&(*o, extractor_for(p).unwrap().1)].clone()))
                .collect();
            let want: BTreeSet<(String, String, String)> = from_scratch(&direct)
                .into_iter()
                .map(|(k, a, b)| (k.as_str().to_owned(), a, b))
                .collect();
            let got: BTreeSet<_> = links_at(&ix.conn, seq as u64).unwrap().into_iter().collect();
            prop_assert_eq!(got, want, "commit {}", seq);
        }
    }
}
