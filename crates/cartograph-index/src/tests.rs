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
