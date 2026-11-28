//! Architecture rules against a hand-built index (XS-22): captures, cycles, levels, liveness, and
//! the scale gate (100 rules over 200k links in under 2 s; run it in an optimised build:
//! `cargo test --profile perf -p cartograph-index --test rules -- --ignored --nocapture`).

use std::collections::BTreeMap;
use std::time::Instant;

use cartograph_discovery::rules::{LinkKind, MinConfidence, Rule, RuleType, Selector, Severity};
use cartograph_index::rules::evaluate;
use cartograph_store::Index;
use cartograph_store::rusqlite::{Connection, params};

struct Db {
    _dir: tempfile::TempDir,
    ix: Index,
}

impl Db {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ix = Index::create(&dir.path().join("i.cartograph")).unwrap();
        ix.conn
            .execute(
                "INSERT INTO repo (id, name, path, shallow) VALUES (1, 'r', '/r', 0)",
                [],
            )
            .unwrap();
        Self { _dir: dir, ix }
    }
    fn conn(&self) -> &Connection {
        &self.ix.conn
    }
    fn service(&self, name: &str) -> i64 {
        self.conn()
            .execute(
                "INSERT INTO node (kind, key, name) VALUES ('service', ?1, ?2)",
                params![format!("service:r:{name}"), name],
            )
            .unwrap();
        self.conn().last_insert_rowid()
    }
    fn file(&self, service: i64, path: &str) -> i64 {
        let c = self.conn();
        c.execute("INSERT INTO file (repo) VALUES (1)", []).unwrap();
        let file = c.last_insert_rowid();
        c.execute(
            "INSERT INTO file_path (file, path, from_seq) VALUES (?1, ?2, 0)",
            params![file, path],
        )
        .unwrap();
        c.execute(
            "INSERT INTO node (kind, file, service) VALUES ('file', ?1, ?2)",
            params![file, service],
        )
        .unwrap();
        c.last_insert_rowid()
    }
    /// A link live from step 0, or ended at step 5.
    fn link(&self, kind: &str, level: Option<&str>, src: i64, dst: i64, live: bool) {
        let c = self.conn();
        c.execute(
            "INSERT INTO link (kind, src, dst, level) VALUES (?1, ?2, ?3, ?4)",
            params![kind, src, dst, level],
        )
        .unwrap();
        c.execute(
            "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, 0, ?2)",
            params![c.last_insert_rowid(), (!live).then_some(5)],
        )
        .unwrap();
    }
}

fn rule(name: &str, kind: RuleType) -> Rule {
    Rule {
        name: name.into(),
        kind,
        severity: Severity::Error,
        from: Selector::default(),
        to: Selector::default(),
        link: vec![],
        min_confidence: MinConfidence::Likely,
        reason: None,
    }
}

fn tags() -> BTreeMap<String, Vec<String>> {
    BTreeMap::new()
}

#[test]
fn a_capture_in_the_near_end_names_the_far_end() {
    let db = Db::new();
    let (a, b) = (db.service("a"), db.service("b"));
    let (ax, bx, by) = (
        db.file(a, "pkg/x/src.ts"),
        db.file(b, "pkg/x/dst.ts"),
        db.file(b, "pkg/y/dst.ts"),
    );
    db.link("http", Some("exact"), ax, bx, true);
    db.link("http", Some("exact"), ax, by, true);
    let mut r = rule("same-package", RuleType::Forbid);
    r.from.path = Some("^pkg/([^/]+)/".into());
    r.to.path = Some("^pkg/$1/".into());
    let v = evaluate(db.conn(), &[r], &tags()).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].from, "a:pkg/x/src.ts");
    assert_eq!(v[0].to, "b:pkg/x/dst.ts");
}

#[test]
fn only_confident_links_are_violations_and_only_live_ones_are_live() {
    let db = Db::new();
    let (a, b) = (db.service("a"), db.service("b"));
    let (f, g) = (db.file(a, "f.ts"), db.file(b, "g.ts"));
    db.link("http", Some("guessed"), f, g, true);
    db.link("db", Some("weak"), f, g, true);
    db.link("import", None, f, g, false);
    db.link("event", Some("likely"), f, g, true);
    let mut r = rule("all", RuleType::Forbid);
    let v = evaluate(db.conn(), &[r.clone()], &tags()).unwrap();
    let kinds: Vec<_> = v.iter().map(|v| (v.kind.as_str(), v.live)).collect();
    assert_eq!(kinds, [("import", false), ("event", true)]);
    assert_eq!(v[0].intervals, [(0, Some(5))]);
    r.min_confidence = MinConfidence::Exact;
    let v = evaluate(db.conn(), &[r], &tags()).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].kind, "import");
}

#[test]
fn selectors_use_manifest_tags_and_the_order_of_rules_does_not_matter() {
    let db = Db::new();
    let (a, b, c) = (db.service("a"), db.service("b"), db.service("c"));
    let (f, g, h) = (db.file(a, "f.ts"), db.file(b, "g.ts"), db.file(c, "h.ts"));
    db.link("http", Some("exact"), f, g, true);
    db.link("http", Some("exact"), g, h, true);
    let tags: BTreeMap<_, _> = [
        ("a".to_owned(), vec!["layer:web".to_owned()]),
        ("c".to_owned(), vec!["layer:data".to_owned()]),
    ]
    .into();
    let mut one = rule("one", RuleType::Forbid);
    one.from.tags = vec!["layer:web".into()];
    let mut two = rule("two", RuleType::Forbid);
    two.to.tags = vec!["layer:data".into()];
    two.link = vec![LinkKind::Http];
    let fwd = evaluate(db.conn(), &[one.clone(), two.clone()], &tags).unwrap();
    let mut rev = evaluate(db.conn(), &[two, one], &tags).unwrap();
    let mut fwd_keys: Vec<_> = fwd.iter().map(|v| v.key()).collect();
    let mut rev_keys: Vec<_> = rev.drain(..).map(|v| v.key()).collect();
    fwd_keys.sort();
    rev_keys.sort();
    assert_eq!(fwd_keys, rev_keys);
    assert_eq!(fwd_keys.len(), 2);
}

#[test]
fn no_cycles_names_the_links_inside_a_cycle() {
    let db = Db::new();
    let (a, b, c) = (db.service("a"), db.service("b"), db.service("c"));
    let (f, g, h) = (db.file(a, "f.ts"), db.file(b, "g.ts"), db.file(c, "h.ts"));
    db.link("http", Some("exact"), f, g, true);
    db.link("http", Some("likely"), g, f, true);
    db.link("http", Some("exact"), g, h, true);
    let v = evaluate(db.conn(), &[rule("acyclic", RuleType::NoCycles)], &tags()).unwrap();
    let mut ends: Vec<_> = v.iter().map(|v| (v.from.as_str(), v.to.as_str())).collect();
    ends.sort();
    assert_eq!(ends, [("a", "b"), ("b", "a")]);
}

#[test]
#[ignore = "scale gate: run in an optimised build with --ignored"]
fn a_hundred_rules_over_two_hundred_thousand_links() {
    let db = Db::new();
    let c = db.conn();
    c.execute_batch("BEGIN").unwrap();
    let services: Vec<i64> = (0..60).map(|i| db.service(&format!("s{i}"))).collect();
    let files: Vec<i64> = (0..6_000)
        .map(|i| db.file(services[i % 60], &format!("src/{}/f{i}.ts", i % 60)))
        .collect();
    // A cheap deterministic spread; one link in a hundred is a table read.
    let mut x: u64 = 12345;
    let mut next = |n: usize| {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 33) as usize) % n
    };
    let mut st = c
        .prepare("INSERT INTO link (kind, src, dst, level) VALUES (?1, ?2, ?3, 'likely')")
        .unwrap();
    let mut iv = c
        .prepare("INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, 0, NULL)")
        .unwrap();
    let mut made = 0;
    while made < 200_000 {
        let (s, d) = (files[next(files.len())], files[next(files.len())]);
        let kind = if next(100) == 0 { "db" } else { "http" };
        if st.execute(params![kind, s, d]).is_ok() {
            iv.execute([c.last_insert_rowid()]).unwrap();
            made += 1;
        }
    }
    drop((st, iv));
    c.execute_batch("COMMIT").unwrap();

    let tag_of: BTreeMap<String, Vec<String>> = (0..60)
        .map(|i| (format!("s{i}"), vec![format!("group:{}", i % 6)]))
        .collect();
    let mut rules = Vec::new();
    for i in 0..100 {
        let mut r = rule(&format!("r{i}"), RuleType::Forbid);
        r.from.tags = vec![format!("group:{}", i % 6)];
        r.to.service = Some(format!("s{}", i % 60));
        r.link = vec![LinkKind::Db];
        if i % 3 == 0 {
            // A path rule: it reads paths only for links the service sets let through.
            r.from.path = Some(r"^src/(\d+)/".into());
            r.to.path = Some(r"^src/$1/".into());
        }
        rules.push(r);
    }
    let t = Instant::now();
    let v = evaluate(c, &rules, &tag_of).unwrap();
    let took = t.elapsed();
    println!("100 rules x 200k links: {took:?}, {} violations", v.len());
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
}
