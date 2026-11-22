//! IDX-05's done-when check on remotion: the tree-sitter specifiers per blob are a superset of
//! bench.mjs's regular-expression output, minus its false matches in strings and comments, and a
//! second run reads 100% from the cache. Run with
//! `CARTOGRAPH_REMOTION=<clone> cargo test --release -p cartograph-index --test extract_remotion -- --ignored --nocapture`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use cartograph_facts::FactCache;
use cartograph_git::{ObjectId, Repo, read_blob};
use cartograph_index::extract::{Facts, extract};
use cartograph_parse::{BlobFacts, Fact, MAX_BLOB_BYTES, extractor_for, tracked};

/// bench.mjs's IMPORT pattern.
const IMPORT: &str = r#"(?:\bimport\s+(?:[^'"()]*?\sfrom\s+)?|\bexport\s+[^'"()]*?\sfrom\s+|\brequire\s*\(\s*|\bimport\s*\(\s*)['"]([^'"\n]+)['"]"#;

fn language(kind: &str) -> tree_sitter::Language {
    match kind {
        "ts-imports/ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "ts-imports/tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

/// Why the regular expression matched where the parser found no import.
fn classify(kind: &str, src: &[u8], at: usize) -> &'static str {
    let mut p = tree_sitter::Parser::new();
    p.set_language(&language(kind)).unwrap();
    let tree = p.parse(src, None).unwrap();
    let mut n = tree.root_node().descendant_for_byte_range(at, at);
    while let Some(x) = n {
        match x.kind() {
            "comment" | "html_comment" => return "false match: inside a comment",
            "template_string" => return "false match: inside a template string",
            "string" => return "false match: inside a string",
            "regex" => return "false match: inside a regular expression",
            "jsx_text" => return "false match: JSX text",
            "ERROR" => return "parse error in the blob",
            _ => n = x.parent(),
        }
    }
    "unexplained"
}

#[test]
#[ignore = "needs CARTOGRAPH_REMOTION pointing at a remotion clone"]
fn remotion_specifiers_are_a_superset_of_the_regex() {
    let path = std::env::var("CARTOGRAPH_REMOTION").expect("CARTOGRAPH_REMOTION");
    let repo = Repo::open(path.as_ref()).unwrap();
    let to = repo.resolve("2b1b0c9").unwrap();
    let mut items: BTreeSet<(ObjectId, &'static str)> = BTreeSet::new();
    for s in repo.walk_first_parent(None, to, tracked).unwrap() {
        for c in s.unwrap().changes {
            if let (Some(n), Some((_, k))) = (c.new, extractor_for(&c.path)) {
                items.insert((n, k));
            }
        }
    }
    let home = tempfile::tempdir().unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)))
        .build()
        .unwrap();
    let cache_path = home.path().join("cache/facts.db");
    let mut facts = Facts::new();
    let t = Instant::now();
    let cold = extract(
        &repo,
        items.iter().copied(),
        &mut FactCache::open(&cache_path).unwrap(),
        &pool,
        &mut facts,
    )
    .unwrap();
    let cold_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let mut again = Facts::new();
    let warm = extract(
        &repo,
        items.iter().copied(),
        &mut FactCache::open(&cache_path).unwrap(),
        &pool,
        &mut again,
    )
    .unwrap();
    let warm_ms = t.elapsed().as_millis();
    println!("cold {cold:?} in {cold_ms} ms; warm {warm:?} in {warm_ms} ms");
    assert_eq!((warm.parsed, warm.from_cache), (0, warm.blobs));
    assert_eq!(facts, again);

    let re = regex::Regex::new(IMPORT).unwrap();
    let local = repo.local();
    let mut classes: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut note = |class: String, example: String| {
        let e = classes.entry(class).or_default();
        e.0 += 1;
        if e.1.len() < 3 {
            e.1.push(example);
        }
    };
    let (mut blobs, mut regex_specs, mut ts_specs) = (0, 0, 0);
    for (&(oid, kind), f) in &facts {
        if !kind.starts_with("ts-imports") {
            continue;
        }
        blobs += 1;
        let Ok(bytes) = read_blob(&local, oid, MAX_BLOB_BYTES) else {
            continue; // bench.mjs skips blobs over 1 MB too
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut by_regex: BTreeMap<String, usize> = BTreeMap::new();
        for c in re.captures_iter(&text) {
            let m = c.get(1).unwrap();
            by_regex.entry(m.as_str().to_owned()).or_insert(m.start());
        }
        let by_ts: BTreeSet<String> = f
            .facts()
            .iter()
            .filter_map(|f| match f {
                Fact::Import { spec, .. } => Some(spec.clone()),
                _ => None,
            })
            .collect();
        regex_specs += by_regex.len();
        ts_specs += by_ts.len();
        for (spec, at) in &by_regex {
            if by_ts.contains(spec) {
                continue;
            }
            let class = match f.as_ref() {
                BlobFacts::Skipped(r) => format!("blob skipped: {r:?}"),
                BlobFacts::Facts(_) => classify(kind, &bytes, *at).to_owned(),
            };
            note(
                format!("regex only, {class}"),
                format!("{} {spec:?}", &oid.to_string()[..10]),
            );
        }
        for spec in by_ts.iter().filter(|s| !by_regex.contains_key(*s)) {
            note(
                "parser only".into(),
                format!("{} {spec:?}", &oid.to_string()[..10]),
            );
        }
    }
    println!(
        "source blobs {blobs}; distinct specifiers per blob: regex {regex_specs}, parser {ts_specs}"
    );
    for (class, (n, ex)) in &classes {
        println!("  {n:>6}  {class}  e.g. {}", ex.join(", "));
    }
    assert!(
        !classes.keys().any(|c| c.contains("unexplained")),
        "every regex-only specifier is explained"
    );
}
