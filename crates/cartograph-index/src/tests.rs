use std::collections::BTreeSet;

use cartograph_facts::FactCache;
use cartograph_git::Repo;
use cartograph_git::fixture::Fixture;
use cartograph_parse::extractor_for;

use crate::extract::{Facts, extract};

fn pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap()
}

#[test]
fn a_second_run_reads_everything_from_the_cache() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("src/a.ts", "import b from './b';");
    f.write("src/b.ts", "export default 1;");
    f.write("package.json", "{\"name\":\"x\"}");
    f.write("README.md", "not tracked");
    f.commit("one");
    f.write("src/b.ts", "export default 2; // @generated\n");
    f.write("big.js", &"x".repeat(1_000_001));
    f.commit("two");
    let repo = Repo::open(&f.dir).unwrap();
    let items: BTreeSet<_> = repo
        .walk_first_parent(None, repo.head().unwrap(), cartograph_parse::tracked)
        .unwrap()
        .flat_map(|s| s.unwrap().changes)
        .filter_map(|c| Some((c.new?, extractor_for(&c.path)?.1)))
        .collect();
    let cache_path = d.path().join("home/cache/facts.db");
    let run = || {
        let mut cache = FactCache::open(&cache_path).unwrap();
        let mut facts = Facts::new();
        let st = extract(
            &repo,
            items.iter().copied(),
            &mut cache,
            &pool(),
            &mut facts,
        )
        .unwrap();
        (st, facts)
    };
    let (first, facts1) = run();
    assert_eq!((first.blobs, first.parsed, first.from_cache), (5, 5, 0));
    assert_eq!(first.skipped, 2, "the generated file and the oversized one");
    let (second, facts2) = run();
    assert_eq!((second.blobs, second.parsed, second.from_cache), (5, 0, 5));
    assert_eq!(facts1, facts2);
}

/// IDX-07: a fixture history indexed end to end, with a golden dump of every interval joined to
/// the commits that opened and closed it, and the as-of oracle at every commit.
#[test]
fn end_to_end_golden_intervals_and_the_as_of_oracle() {
    use cartograph_facts::from_scratch;
    use cartograph_store::Index;

    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("package.json", r#"{"name":"root","private":true}"#);
    f.write(
        "src/a.ts",
        "import { b } from './b';\nimport l from 'lib';\n",
    );
    f.commit("add a");
    f.write("src/b/index.ts", "export const b = 1;\n");
    f.write(
        "lib/package.json",
        r#"{"name":"lib","exports":"./src/index.ts"}"#,
    );
    f.write("lib/src/index.ts", "export default 1;\n");
    f.commit("add b and lib");
    f.write("src/b.ts", "export const b = 2;\n");
    f.commit("a better candidate");
    f.write(
        "lib/package.json",
        r#"{"name":"lib","version":"2.0.0","exports":"./src/index.ts"}"#,
    );
    f.commit("version bump");
    f.write(
        "lib/package.json",
        r#"{"name":"other","main":"dist/index.js"}"#,
    );
    f.write(
        "src/c.ts",
        "import a from './a.js';\nimport o from 'other';\n",
    );
    f.commit("rename the package");
    f.remove("src/b.ts");
    f.commit("delete b");

    let out = d.path().join("home/workspaces/w/index.cartograph");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut cache = FactCache::open(&d.path().join("home/cache/facts.db")).unwrap();
    let s = crate::build_single(&f.dir, None, &out, &mut cache, 2).unwrap();
    assert_eq!(s.steps, 6);
    assert!(matches!(
        crate::build_single(&f.dir, None, &out, &mut cache, 2),
        Err(crate::BuildError::Store(
            cartograph_store::StoreError::Exists { .. }
        ))
    ));

    let ix = Index::open_read(&out).unwrap();
    let mut st = ix
        .conn
        .prepare(
            "SELECT l.kind, ps.path, pd.path, i.from_seq, co.summary, i.to_seq, cc.summary
             FROM interval i JOIN link l ON l.id = i.link
             JOIN node ns ON ns.id = l.src JOIN file_path ps ON ps.file = ns.file
             JOIN node nd ON nd.id = l.dst JOIN file_path pd ON pd.file = nd.file
             JOIN commits co ON co.seq = i.from_seq LEFT JOIN commits cc ON cc.seq = i.to_seq
             GROUP BY i.rowid ORDER BY ps.path, pd.path, i.from_seq",
        )
        .unwrap();
    let dump: Vec<String> = st
        .query_map([], |r| {
            Ok(format!(
                "{} {} -> {}  opened {} ({})  closed {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<String>>(6)?) {
                    (Some(t), Some(m)) => format!("{t} ({m})"),
                    _ => "never".into(),
                }
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    insta::assert_snapshot!(dump.join("\n"));

    // The as-of oracle: at every commit, the links read from the file equal a from-scratch index.
    let repo = Repo::open(&f.dir).unwrap();
    let (steps, facts, ..) =
        crate::build::walk_and_extract(&repo, repo.head().unwrap(), &mut cache, 2).unwrap();
    for (seq, step) in steps.iter().enumerate() {
        let tree = repo.tree_at(step.sha, cartograph_parse::tracked).unwrap();
        let direct = tree
            .iter()
            .map(|(p, o)| (p.clone(), facts[&(*o, extractor_for(p).unwrap().1)].clone()))
            .collect();
        let want: Vec<(String, String, String)> = from_scratch(&direct)
            .into_iter()
            .map(|(k, a, b)| (k.as_str().to_owned(), a, b))
            .collect();
        assert_eq!(
            crate::build::links_at(&ix.conn, seq as u64).unwrap(),
            want,
            "step {seq}"
        );
    }
}
