//! XS-12: the overrides file bound to the index and applied to links. A correction survives the
//! rename of its caller, goes stale when its handler is deleted, and applying is idempotent.

use std::time::Instant;

use cartograph_discovery::overrides::{self, FILE_NAME};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::corrections::{self, Blame};
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;
use cartograph_store::rusqlite;
use proptest::prelude::*;

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\napp.get(\"/legacy\", (req, res) => res.json({}));\n";
const LEGACY: &str = "import express from \"express\";\nconst app = express();\napp.get(\"/old\", (req, res) => res.json({}));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
const OTHER: &str = "export function old() {\n  return fetch(\"http://orders:8080/legacy\");\n}\n";

struct Env {
    _d: tempfile::TempDir,
    f: Fixture,
    w: Workspace,
    cache: FactCache,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("orders/src/legacy.ts", LEGACY);
    f.write("web/src/api.ts", API);
    f.write("web/src/other.ts", OTHER);
    f.commit("both services");
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("orders"), svc("web")],
        overrides: Some(f.dir.join(FILE_NAME)),
        ..Default::default()
    };
    let cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    Env { _d: d, f, w, cache }
}

impl Env {
    fn sync(&mut self) {
        match sync(&self.w, &SyncOptions::default(), &mut self.cache).unwrap() {
            SyncOutcome::Synced(_) => {}
            SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
        }
    }
    fn index(&self) -> Index {
        Index::open_read(&self.w.index_path()).unwrap()
    }
    /// Every HTTP link: `from -> to level score layer intervals`, its base values when changed.
    fn links(&self) -> Vec<String> {
        dump(&self.index().conn)
    }
}

fn dump(c: &rusqlite::Connection) -> Vec<String> {
    let path = |n: &str| {
        format!(
            "(SELECT path FROM file_path p WHERE p.file = {n}.file ORDER BY from_seq DESC LIMIT 1)"
        )
    };
    let mut st = c
        .prepare(&format!(
            "SELECT {}, {}, l.level, l.score, coalesce(l.layer, 'base'),
                    coalesce((SELECT group_concat(i.from_seq || '-' || coalesce(i.to_seq, 'open'), ',')
                              FROM interval i WHERE i.link = l.id), 'hidden'),
                    coalesce((SELECT 'was ' || b.level || ' ' || b.score FROM correction_base b
                              WHERE b.link = l.id AND b.added = 0), '')
             FROM link l JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
             WHERE l.kind = 'http' ORDER BY 1, 2",
            path("ns"),
            path("nd")
        ))
        .unwrap();
    st.query_map([], |r| {
        Ok(format!(
            "{} -> {} {} {} {} [{}] {}",
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn statuses(e: &Env) -> Vec<String> {
    corrections::rows(&e.index().conn)
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{}:{} {} {} links={}",
                r.line, r.action, r.key, r.status, r.links
            )
        })
        .collect()
}

const SET: &str = "version: 1
corrections:
  - action: set
    key: {kind: http, from: 'web:web/src/api.ts', to: 'orders:orders/src/server.ts', op: 'GET /orders/{id}'}
    confidence: exact
    reason: the contract says so
";

#[test]
fn golden_file_and_index_to_effective_links() {
    let mut e = env();
    e.sync();
    assert_eq!(
        e.links(),
        [
            "web/src/api.ts -> orders/src/server.ts likely 0.89 base [0-open] ",
            "web/src/other.ts -> orders/src/server.ts likely 0.89 base [0-open] ",
        ]
    );
    e.f.write(
        FILE_NAME,
        "version: 1
corrections:
  - action: set
    key: {kind: http, from: 'web:web/src/api.ts', to: 'orders:orders/src/server.ts', op: 'GET /orders/{id}'}
    confidence: exact
    reason: the contract says so
  - action: retarget
    key: {kind: http, from: web, to: orders, op: GET /legacy}
    to: 'orders:orders/src/legacy.ts'
    reason: the route moved
  - action: add
    key: {kind: http, from: 'web:web/src/other.ts', to: 'orders:orders/src/legacy.ts'}
    confidence: guessed
    reason: seen at runtime
  - action: remove
    key: {kind: http, from: 'web:web/src/api.ts', to: orders, op: GET /nothing}
    reason: not a call
",
    );
    e.f.commit("corrections");
    e.sync();
    insta::assert_snapshot!(
        "effective_links",
        format!("{}\n--\n{}", e.links().join("\n"), statuses(&e).join("\n"))
    );
}

#[test]
fn a_correction_follows_its_caller_through_a_rename() {
    let mut e = env();
    e.f.write(FILE_NAME, SET);
    e.f.commit("a correction");
    e.sync();
    assert!(
        e.links()[0].contains("likely 0.89 base") || e.links()[0].contains("exact 0.95 correction")
    );
    assert_eq!(
        e.links()[0],
        "web/src/api.ts -> orders/src/server.ts exact 0.95 correction [0-open] was likely 0.89"
    );
    e.f.git(&["mv", "web/src/api.ts", "web/src/client.ts"]);
    e.f.commit("rename the caller");
    e.sync();
    assert_eq!(
        e.links()[0],
        "web/src/client.ts -> orders/src/server.ts exact 0.95 correction [0-open] was likely 0.89"
    );
    assert_eq!(
        statuses(&e)[0],
        "3:set web:web/src/api.ts -> orders:orders/src/server.ts GET /orders/{id} bound links=1"
    );
    let by: Option<String> = e
        .index()
        .conn
        .query_row("SELECT by FROM correction", [], |r| r.get(0))
        .unwrap();
    assert!(by.is_some(), "the blame names who added it");
}

#[test]
fn deleting_the_handler_makes_the_entry_stale() {
    let mut e = env();
    e.f.write(FILE_NAME, SET);
    e.f.commit("a correction");
    e.sync();
    assert!(statuses(&e)[0].ends_with("bound links=1"));
    e.f.git(&["rm", "-q", "orders/src/server.ts"]);
    e.f.commit("drop the handler");
    e.sync();
    assert!(
        statuses(&e)[0].ends_with("stale links=1"),
        "{:?}",
        statuses(&e)
    );
}

#[test]
fn removing_the_entry_restores_the_link_and_an_uncommitted_one_anchors_at_the_tip() {
    let mut e = env();
    e.sync();
    let base = e.links();
    e.f.write(
        FILE_NAME,
        &SET.replace("action: set", "action: remove")
            .replace("    confidence: exact\n", ""),
    );
    e.sync();
    assert_eq!(
        e.links()[0],
        "web/src/api.ts -> orders/src/server.ts likely 0.89 correction [hidden] was likely 0.89"
    );
    let (anchor, by): (i64, Option<String>) = e
        .index()
        .conn
        .query_row("SELECT anchor_seq, by FROM correction", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((anchor, by), (0, None));
    e.f.write(FILE_NAME, "version: 1\ncorrections: []\n");
    e.sync();
    assert_eq!(e.links(), base);
    let left: i64 = e
        .index()
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM correction) + (SELECT count(*) FROM correction_base)
                  + (SELECT count(*) FROM correction_interval)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn an_unchanged_file_costs_nothing() {
    let mut e = env();
    e.f.write(FILE_NAME, SET);
    e.f.commit("a correction");
    e.sync();
    let key = |e: &Env| -> Vec<u8> {
        e.index()
            .conn
            .query_row(
                "SELECT inputs_key FROM derived WHERE kind = 'corrections'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let k = key(&e);
    // A mark that a re-apply would wipe: the entry rows are rebuilt only when the key changes.
    let w = Index::open_write(&e.w.index_path()).unwrap();
    w.conn
        .execute("UPDATE correction SET reason = 'marked'", [])
        .unwrap();
    drop(w);
    e.sync();
    let reason: String = e
        .index()
        .conn
        .query_row("SELECT reason FROM correction", [], |r| r.get(0))
        .unwrap();
    assert_eq!((reason.as_str(), key(&e)), ("marked", k));
}

// ---- a synthetic index: nodes and links written directly -------------------------------------

/// `services` services of `files` files each, one open `http` link from every file to every file
/// of the next `fanout` files (by position), each with one evidence row naming its operation
/// `get /o{id}/{}`. Returns the link count.
fn synthetic(c: &rusqlite::Connection, services: i64, files: i64, fanout: i64) -> i64 {
    c.execute_batch("BEGIN").unwrap();
    c.execute(
        "INSERT INTO repo (id, name, path, shallow) VALUES (1, 'r', '/r', 0)",
        [],
    )
    .unwrap();
    let n = services * files;
    for s in 0..services {
        c.execute(
            "INSERT INTO node (id, kind, key, name, service) VALUES (?1, 'service', ?2, ?3, ?1)",
            (1_000_000 + s, format!("service:r:s{s}"), format!("s{s}")),
        )
        .unwrap();
    }
    for i in 0..n {
        let s = i / files;
        c.execute("INSERT INTO file (id, repo) VALUES (?1, 1)", [i + 1])
            .unwrap();
        c.execute(
            "INSERT INTO file_path (file, path, from_seq, to_seq) VALUES (?1, ?2, 0, NULL)",
            (i + 1, format!("s{s}/f{}.ts", i % files)),
        )
        .unwrap();
        c.execute(
            "INSERT INTO node (id, kind, file, service) VALUES (?1, 'file', ?1, ?2)",
            (i + 1, 1_000_000 + s),
        )
        .unwrap();
    }
    let mut links = 0;
    let mut link = c.prepare("INSERT INTO link (id, kind, src, dst, level, score) VALUES (?1, 'http', ?2, ?3, 'likely', 0.8)").unwrap();
    let mut iv = c
        .prepare("INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, 0, NULL)")
        .unwrap();
    let mut ev = c.prepare("INSERT INTO evidence (link, file, kind, text, start_line, start_col, end_line, end_col, extractor, operation) VALUES (?1, ?2, 'literal', 't', 1, 0, 1, 1, 'x', ?3)").unwrap();
    for src in 0..n {
        for k in 1..=fanout {
            let dst = (src + k * 7) % n;
            links += 1;
            link.execute((links, src + 1, dst + 1)).unwrap();
            iv.execute([links]).unwrap();
            ev.execute((links, src + 1, format!("get /o{links}/{{}}")))
                .unwrap();
        }
    }
    c.execute_batch("COMMIT").unwrap();
    links
}

fn new_index() -> (tempfile::TempDir, Index) {
    let d = tempfile::tempdir().unwrap();
    let ix = Index::create(&d.path().join("i.cartograph")).unwrap();
    (d, ix)
}

fn parse(entries: &str) -> overrides::Parsed {
    overrides::parse(&format!("version: 1\ncorrections:\n{entries}")).unwrap()
}

fn entry(action: &str, src: i64, dst: i64, link: i64, files: i64, extra: &str) -> String {
    let end = |i: i64| format!("s{}:s{}/f{}.ts", i / files, i / files, i % files);
    format!(
        "  - action: {action}\n    key: {{kind: http, from: '{}', to: '{}', op: 'GET /o{link}/{{id}}'}}\n{extra}    reason: r\n",
        end(src),
        end(dst)
    )
}

#[test]
fn perf_a_thousand_corrections_against_two_hundred_thousand_links() {
    let (_d, ix) = new_index();
    let (services, files, fanout) = (20, 500, 20);
    let links = synthetic(&ix.conn, services, files, fanout);
    assert_eq!(links, 200_000);
    let n = services * files;
    let mut text = String::new();
    for i in 0..1000 {
        let (src, k) = ((i * 9) % n, 1 + i % fanout);
        let dst = (src + k * 7) % n;
        let link = src * fanout + k;
        let action = ["set", "remove"][(i % 2) as usize];
        let extra = if action == "set" {
            "    confidence: exact\n"
        } else {
            ""
        };
        text += &entry(action, src, dst, link, files, extra);
    }
    let p = parse(&text);
    // As in a sync: inside the run's transaction.
    ix.conn.execute_batch("BEGIN").unwrap();
    let t = Instant::now();
    let bound = corrections::apply(&ix.conn, &p, &[], 0).unwrap();
    let took = t.elapsed();
    ix.conn.execute_batch("COMMIT").unwrap();
    assert_eq!(bound, 1000);
    println!("perf,corrections_1000_vs_200k,{},200,ms", took.as_millis());
    // Wall-clock gates only in an optimised build, as the other perf gates (CI's perf job).
    if !cfg!(debug_assertions) {
        assert!(took.as_millis() < 200, "{took:?}");
    }
}

fn state(c: &rusqlite::Connection) -> Vec<String> {
    let mut v: Vec<String> = c
        .prepare(
            "SELECT 'l' || l.id || ' ' || l.src || ' ' || l.dst || ' ' || coalesce(l.level, '-') || ' '
                    || coalesce(l.score, 0) || ' ' || coalesce(l.layer, '-') || ' '
                    || coalesce((SELECT group_concat(i.from_seq || '-' || coalesce(i.to_seq, 'open'), ',')
                                 FROM interval i WHERE i.link = l.id), '')
             FROM link l ORDER BY l.id",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    v.sort();
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Applying is idempotent, and applying nothing after anything restores the base links.
    #[test]
    fn apply_is_idempotent_and_clearing_restores_the_base(
        picks in proptest::collection::vec((0usize..5, 0i64..40, 1i64..4, 0i64..40), 0..24),
    ) {
        let (_d, ix) = new_index();
        let (services, files, fanout) = (4, 10, 3);
        synthetic(&ix.conn, services, files, fanout);
        let n = services * files;
        let base = state(&ix.conn);
        let mut text = String::new();
        for (a, src, k, other) in picks {
            let dst = (src + k * 7) % n;
            let link = src * fanout + k;
            text += &match a {
                0 => entry("set", src, dst, link, files, "    confidence: guessed\n"),
                1 => entry("remove", src, dst, link, files, ""),
                2 => entry("retarget", src, dst, link, files, &format!("    to: 's{}:s{}/f{}.ts'\n", other / files, other / files, other % files)),
                3 => entry("add", src, other, link, files, ""),
                _ => entry("set", src, dst, link, files, "    confidence: likely\n"),
            };
        }
        let p = parse(&text);
        corrections::apply(&ix.conn, &p, &[Blame::default()], 0).unwrap();
        let once = state(&ix.conn);
        corrections::apply(&ix.conn, &p, &[], 0).unwrap();
        prop_assert_eq!(&once, &state(&ix.conn));
        corrections::clear(&ix.conn).unwrap();
        prop_assert_eq!(base, state(&ix.conn));
    }
}

#[test]
fn the_path_never_names_a_place_outside_the_index() {
    let (_d, ix) = new_index();
    synthetic(&ix.conn, 2, 3, 1);
    // `..` is refused by the reader; a path the index doesn't have binds to nothing.
    assert!(overrides::parse("version: 1\ncorrections:\n  - action: remove\n    key: {kind: http, from: 's0:../../etc/passwd', to: s1, op: GET /}\n    reason: r\n").is_err());
    let p = parse(
        "  - action: remove\n    key: {kind: http, from: 's0:etc/passwd', to: s1, op: GET /}\n    reason: r\n",
    );
    corrections::apply(&ix.conn, &p, &[], 0).unwrap();
    let rows = corrections::rows(&ix.conn).unwrap();
    assert_eq!((rows[0].status.as_str(), rows[0].links), ("stale", 0));
}

// ---- XS-13: the writer --------------------------------------------------------------------

use cartograph_discovery::overrides::{Action, Entry, Key, Kind, Level};
use cartograph_index::corrections::Why;

fn key(op: Option<&str>) -> Key {
    Key {
        kind: Kind::Http,
        from: "web:web/src/api.ts".into(),
        to: "orders:orders/src/server.ts".into(),
        op: op.map(Into::into),
        topic: None,
        table: None,
    }
}

fn hide(reason: &str) -> Entry {
    Entry {
        action: Action::Remove,
        key: key(Some("GET /orders/{id}")),
        to: None,
        confidence: None,
        reason: reason.into(),
    }
}

fn fix(level: Level) -> Entry {
    Entry {
        action: Action::Set,
        confidence: Some(level),
        ..hide("the contract says so")
    }
}

fn written(e: &Env) -> String {
    std::fs::read_to_string(e.f.dir.join(FILE_NAME)).unwrap_or_default()
}

#[test]
fn append_and_undo_through_the_index_with_golden_diffs() {
    let mut e = env();
    e.sync();
    let base = e.links();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;

    // A dry run says what would happen and writes nothing.
    let dry = corrections::append(&path, &conn, &fix(Level::Exact), true).unwrap();
    assert!(!dry.applied && !path.exists());
    let d = corrections::append(&path, &conn, &fix(Level::Exact), false).unwrap();
    assert_eq!(d.diff, dry.diff);
    insta::assert_snapshot!("append_creates_the_file", d.diff);
    insta::assert_snapshot!("append_created_file", written(&e));
    e.sync();
    assert_eq!(
        e.links()[0],
        "web/src/api.ts -> orders/src/server.ts exact 0.95 correction [0-open] was likely 0.89"
    );

    // A second entry goes after the first, and the file's comments stay where they are.
    let text = format!("# kept\n{}# also kept\n", written(&e));
    std::fs::write(&path, &text).unwrap();
    let conn = e.index().conn;
    let d = corrections::append(&path, &conn, &hide("not a call"), false).unwrap();
    insta::assert_snapshot!("append_after_comments", d.diff);
    assert!(written(&e).starts_with(&text));
    e.sync();
    assert_eq!(
        statuses(&e),
        [
            "4:set web:web/src/api.ts -> orders:orders/src/server.ts GET /orders/{id} bound links=1",
            "9:remove web:web/src/api.ts -> orders:orders/src/server.ts GET /orders/{id} bound links=1",
        ]
    );

    // Undo the second, then the first: the file and the links are as they were.
    let d = corrections::remove(&path, 1, &fp(&path, 1), false).unwrap();
    insta::assert_snapshot!("undo_the_second", d.diff);
    assert_eq!(written(&e), text);
    corrections::remove(&path, 0, &fp(&path, 0), false).unwrap();
    assert_eq!(
        written(&e),
        "# kept\nversion: 1\ncorrections:\n# also kept\n"
    );
    e.sync();
    assert_eq!(e.links(), base);
    assert_eq!(
        corrections::remove(&path, 0, "00000000", false)
            .unwrap_err()
            .why,
        Why::Missing
    );
}

/// The fingerprint of entry `i` of the file, as the API's id carries it.
fn fp(path: &std::path::Path, i: usize) -> String {
    let parsed = overrides::read(path).unwrap().unwrap().1.unwrap();
    parsed.overrides.corrections[i].fingerprint()
}

/// An id names an entry by position and content: an undo after the file changed removes that
/// entry where it moved to, or nothing, never a neighbour.
#[test]
fn undo_removes_the_entry_its_id_names_or_nothing() {
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;
    corrections::append(&path, &conn, &fix(Level::Exact), false).unwrap();
    let (at, kept) = (0, fp(&path, 0));
    // Someone puts another entry first: position 0 is now a different entry.
    let text = written(&e);
    let other = "  - action: remove\n    key: {kind: http, from: 'web:web/src/other.ts', to: 'orders:orders/src/legacy.ts', op: 'GET /legacy'}\n    reason: \"theirs\"\n";
    let moved = text.replacen("corrections:\n", &format!("corrections:\n{other}"), 1);
    std::fs::write(&path, &moved).unwrap();
    corrections::remove(&path, at, &kept, false).unwrap();
    let left = overrides::read(&path).unwrap().unwrap().1.unwrap();
    assert_eq!(left.overrides.corrections.len(), 1);
    assert_eq!(left.overrides.corrections[0].reason, "theirs");
    // The entry is edited by hand: its old id removes nothing.
    let edited = written(&e).replace("\"theirs\"", "\"edited\"");
    std::fs::write(&path, &edited).unwrap();
    let theirs = fp(&path, 0);
    let stale = format!("{}0", &theirs[..7]);
    assert_eq!(
        corrections::remove(&path, 0, &stale, false)
            .unwrap_err()
            .why,
        Why::Missing
    );
    assert_eq!(written(&e), edited);
}

#[test]
fn the_file_is_appended_to_never_rewritten() {
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    // Odd spacing, a quoted key, a trailing comment and no final newline all survive.
    let old = "version: 1   # the format\nstale:   warn\ncorrections:\n\n  # why\n- action: remove\n  key: {kind: http, from: 'web:web/src/other.ts', to: 'orders:orders/src/legacy.ts', op: \"GET /legacy\"}\n  reason:    'old'\n\n# end";
    std::fs::write(&path, old).unwrap();
    let conn = e.index().conn;
    let d = corrections::append(&path, &conn, &fix(Level::Likely), false).unwrap();
    insta::assert_snapshot!("append_to_an_untidy_file", d.diff);
    assert!(written(&e).starts_with(old));
    corrections::remove(&path, 1, &fp(&path, 1), false).unwrap();
    assert_eq!(written(&e), old);
}

#[test]
fn append_then_delete_leaves_the_file_byte_identical() {
    use proptest::test_runner::TestRunner;
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;
    let existing = "  - action: remove\n    key: {kind: http, from: 'web:web/src/other.ts', to: 'orders:orders/src/legacy.ts', op: 'GET /legacy'}\n    reason: old\n";
    let strat = (
        proptest::sample::select(vec![
            "",
            "version: 1\n",
            "version: 1\ncorrections:\n",
            "# top\nversion: 1\nstale: warn\ncorrections:\n  # note\n",
        ]),
        proptest::bool::ANY,
        proptest::sample::select(vec![
            "",
            "\n",
            "# tail\n",
            "\n\n# tail\n  # indented tail\n",
        ]),
        proptest::bool::ANY,
        proptest::sample::select(vec![
            "a reason",
            "with: colons # and \"quotes\" and \\ slashes",
            "ünïcode — ok",
        ]),
        0usize..3,
    );
    let mut runner = TestRunner::default();
    runner
        .run(&strat, |(head, with_entry, tail, trim, reason, which)| {
            let head = if head.contains("corrections:") && with_entry {
                format!("{head}{existing}")
            } else {
                head.to_owned()
            };
            let mut old = format!("{head}{tail}");
            if trim {
                old = old.trim_end_matches('\n').to_owned();
            }
            // A file the reader refuses (no version) isn't written to.
            if !old.is_empty() && overrides::parse(&old).is_err() {
                return Ok(());
            }
            if old.is_empty() {
                let _ = std::fs::remove_file(&path);
            } else {
                std::fs::write(&path, &old).unwrap();
            }
            let entry = [hide(reason), fix(Level::Guessed), fix(Level::Exact)][which].clone();
            let n = corrections::append(&path, &conn, &entry, false).unwrap();
            prop_assert!(n.applied);
            let parsed = overrides::read(&path).unwrap().unwrap().1.unwrap();
            let last = parsed.overrides.corrections.len() - 1;
            prop_assert_eq!(&parsed.overrides.corrections[last], &entry);
            corrections::remove(&path, last, &fp(&path, last), false).unwrap();
            let after = std::fs::read_to_string(&path).unwrap_or_default();
            // `corrections:` is the one thing an append adds to a file that had none (and a file made by
            // the first append is left as `version: 1` and `corrections:`).
            if old.lines().any(|l| l.starts_with("corrections:")) {
                prop_assert_eq!(after, old);
            } else if old.is_empty() {
                prop_assert_eq!(after, "version: 1\ncorrections:\n");
            } else {
                prop_assert_eq!(
                    after.replace("\n\n", "\n").trim_end().to_owned(),
                    format!("{}\ncorrections:", old.trim_end_matches('\n')).replace("\n\n", "\n")
                );
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn text_in_a_reason_stays_data() {
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;
    let ok = [
        "x\" \n  - action: remove",
        "plain # not a comment",
        "x'; DROP TABLE link; --",
        "a: b, c: {d}, [e]",
        "*alias &anchor !!python/object/apply:os.system [id]",
        "- action: remove",
        "key: {kind: http}",
        "\\n and \\u0000 spelled out",
        "'single' and \"double\"",
        "emoji \u{1F600} and \u{202e}rtl",
        "---",
    ];
    // The first has a newline in it, which is refused: the others are kept as written.
    assert!(corrections::append(&path, &conn, &hide(ok[0]), false).is_err());
    for (i, r) in ok[1..].iter().enumerate() {
        corrections::append(&path, &conn, &hide(r), false).unwrap();
        let p = overrides::read(&path).unwrap().unwrap().1.unwrap();
        assert_eq!(p.overrides.corrections.len(), i + 1, "{r:?}");
        assert_eq!(p.overrides.corrections[i].reason, *r);
        assert_eq!(p.overrides.corrections[i].action, Action::Remove);
    }
    let before = written(&e);
    for bad in [
        "line\nbreak",
        "carriage\rreturn",
        "nul\0byte",
        "tab\tseparated",
        "escape\u{1b}[31m",
        "next\u{85}line",
        "line\u{2028}separator",
        "para\u{2029}separator",
        "\u{feff}bom",
        "del\u{7f}",
    ] {
        let err = corrections::append(&path, &conn, &hide(bad), false).unwrap_err();
        assert_eq!(err.why, Why::Invalid, "{bad:?}");
    }
    assert_eq!(written(&e), before, "a refused write changes nothing");
}

#[test]
fn the_index_decides_what_can_be_written() {
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;
    let mut nothing = hide("r");
    nothing.key.op = Some("GET /nothing".into());
    let mut elsewhere = hide("r");
    elsewhere.key.from = "web:../../etc/passwd".into();
    let mut unknown = hide("r");
    unknown.key.to = "ghost".into();
    let mut event = hide("r");
    event.key = Key {
        kind: Kind::Event,
        op: None,
        topic: Some("t".into()),
        ..key(None)
    };
    for bad in [nothing, elsewhere, unknown, event] {
        assert_eq!(
            corrections::append(&path, &conn, &bad, false)
                .unwrap_err()
                .why,
            Why::Invalid,
            "{bad:?}"
        );
    }
    let mut add = hide("r");
    add.action = Action::Add;
    add.key.op = None;
    add.key.from = "web:web/src/other.ts".into();
    add.key.to = "orders:orders/src/legacy.ts".into();
    assert!(corrections::append(&path, &conn, &add, true).is_ok());
    assert!(!path.exists());
}

#[test]
fn a_file_with_merge_markers_or_a_link_or_a_stray_layout_is_left_alone() {
    let mut e = env();
    e.sync();
    let path = e.f.dir.join(FILE_NAME);
    let conn = e.index().conn;
    let cases = [
        "version: 1\ncorrections:\n<<<<<<< HEAD\n  - action: remove\n=======\n>>>>>>> theirs\n",
        "version: 1\ncorrections: []\n",
        "version: 1\ncorrections:\nstale: warn\n",
        "version: 1\ncorrections:\n  - {action: remove, key: {kind: http, from: a, to: b, op: GET /}, reason: r}\nextra: 1\n",
    ];
    for c in cases {
        std::fs::write(&path, c).unwrap();
        let err = corrections::append(&path, &conn, &fix(Level::Exact), false).unwrap_err();
        assert_eq!(err.why, Why::Invalid, "{c}");
        assert_eq!(written(&e), c);
        assert!(corrections::remove(&path, 0, "00000000", false).is_err());
        assert_eq!(written(&e), c);
    }
    // A link where the file should be is never followed or replaced.
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        let target = e.f.dir.join("elsewhere.yaml");
        std::fs::write(&target, "version: 1\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(corrections::append(&path, &conn, &fix(Level::Exact), false).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "version: 1\n");
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn ten_thousand_entries_append_and_undo_with_one_parse_each() {
    let (d, ix) = new_index();
    let (services, files, fanout) = (20, 500, 20);
    synthetic(&ix.conn, services, files, fanout);
    ix.conn
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('watermark_seq', 0)",
            [],
        )
        .unwrap();
    let n = services * files;
    let mut text = String::from("version: 1\ncorrections:\n");
    for i in 0..(overrides::MAX_ENTRIES as i64 - 1) {
        let (src, k) = (i % n, 1 + i % fanout);
        text += &entry(
            "remove",
            src,
            (src + k * 7) % n,
            src * fanout + k,
            files,
            "",
        );
    }
    let path = d.path().join(FILE_NAME);
    std::fs::write(&path, &text).unwrap();
    let new = Entry {
        action: Action::Remove,
        key: Key {
            kind: Kind::Http,
            from: "s0:s0/f0.ts".into(),
            to: "s0:s0/f7.ts".into(),
            op: Some("GET /o1/{id}".into()),
            topic: None,
            table: None,
        },
        to: None,
        confidence: None,
        reason: "last".into(),
    };
    let t = Instant::now();
    corrections::append(&path, &ix.conn, &new, false).unwrap();
    let appended = t.elapsed();
    let last = fp(&path, overrides::MAX_ENTRIES - 1);
    let t = Instant::now();
    corrections::remove(&path, overrides::MAX_ENTRIES - 1, &last, false).unwrap();
    let removed = t.elapsed();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    println!(
        "perf,corrections_append_10k,{},250,ms",
        appended.as_millis()
    );
    println!("perf,corrections_undo_10k,{},250,ms", removed.as_millis());
    if !cfg!(debug_assertions) {
        assert!(
            appended.as_millis() < 250 && removed.as_millis() < 250,
            "{appended:?} {removed:?}"
        );
    }
    // The file is full at the limit.
    corrections::append(&path, &ix.conn, &new, false).unwrap();
    assert_eq!(
        corrections::append(&path, &ix.conn, &new, false)
            .unwrap_err()
            .why,
        Why::Invalid
    );
}
