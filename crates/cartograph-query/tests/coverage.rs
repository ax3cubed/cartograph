//! XS-14: the six coverage rows. Each figure equals a count the test makes on its own, every gap
//! opens a list as long as the gap, and done + unresolved = total on every row at every step.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::contracts::ContractSpec;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, Page, Store};
use cartograph_store::Index;

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\napp.get(\"/health\", (req, res) => res.send(\"ok\"));\n";
const API: &str = "import { nope } from './nope';\nexport function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\nexport function gone() {\n  return fetch(\"http://orders:8080/missing\");\n}\n";
const SPEC: &str = "openapi: 3.0.3
info: {title: Orders}
paths:
  /orders/{id}:
    get:
      operationId: getOrder
    delete:
      operationId: deleteOrder
";

fn workspace(f: &Fixture, dir: &std::path::Path) -> Workspace {
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    Workspace {
        dir: dir.to_owned(),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("orders"), svc("web")],
        contracts: vec![ContractSpec {
            path: "orders/openapi.yaml".into(),
            service: Some("orders".into()),
        }],
        coverage_targets: [("calls".to_owned(), 90.0)].into(),
        ..Default::default()
    }
}

#[test]
fn every_figure_is_a_count_and_every_gap_opens_a_list_as_long_as_it() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.commit("orders: routes");
    f.write("web/src/api.ts", API);
    f.write("web/src/blob.ts", "export const x = '\u{0}\u{0}\u{0}';\n");
    f.commit("web: calls");
    f.write("orders/openapi.yaml", SPEC);
    f.commit("orders: the spec");
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let ix = Index::open_read(&w.index_path()).unwrap();
    let count = |sql: &str| ix.conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    let live = "to_seq IS NULL";

    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let rows = snap.coverage(&At::Now).unwrap();
    let fig = |id: &str| {
        let r = rows.iter().find(|r| r.id == id).unwrap();
        (r.done as i64, r.total as i64)
    };

    // Independent counts, straight from the tables.
    let ops = count(&format!("SELECT count(*) FROM operation WHERE {live}"));
    let handled = count(&format!(
        "SELECT count(DISTINCT operation) FROM binding WHERE kind = 'handled_by' AND {live}"
    ));
    let consumed = count(&format!(
        "SELECT count(DISTINCT b.operation) FROM binding b WHERE b.kind = 'consumes' AND {live}
           AND b.operation IN (SELECT operation FROM binding WHERE kind = 'handled_by' AND {live})"
    ));
    let routes = count(&format!(
        "SELECT count(*) FROM (SELECT DISTINCT file, line FROM binding
           WHERE kind IN ('handled_by', 'undocumented') AND {live})"
    ));
    let documented = count(&format!(
        "SELECT count(*) FROM (SELECT DISTINCT file, line FROM binding
           WHERE kind = 'handled_by' AND {live})"
    ));
    assert_eq!(
        (ops, handled, consumed, routes, documented),
        (2, 1, 1, 2, 1)
    );
    assert_eq!(fig("handlers"), (handled, ops));
    assert_eq!(fig("routes"), (documented, routes));
    assert_eq!(fig("consumers"), (consumed, handled));
    // Two call sites, one of them to a path no route has.
    let bad_calls = count(&format!(
        "SELECT count(*) FROM unresolved WHERE fact_kind = 'http.call' AND {live}"
    ));
    assert_eq!((bad_calls, fig("calls")), (1, (1, 2)));
    // The imports and files of the run.
    let (ft, fp, it, ir) = ix
        .conn
        .query_row(
            "SELECT files_total, files_parsed, imports_total, imports_resolved FROM run
             ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((fig("files"), fig("imports")), ((fp, ft), (ir, it)));
    assert!(ft > fp, "the file with NULs is skipped");

    // Every gap opens a list as long as the gap; pages of two give the same rows.
    for r in &rows {
        let gap = (r.total - r.done) as usize;
        let all = snap.unresolved(&r.id, &At::Now, Page::default()).unwrap();
        assert_eq!(all.rows.len(), gap, "{}", r.id);
        assert!(all.next_cursor.is_none());
        assert_eq!(r.gaps.iter().map(|g| g.rows as usize).sum::<usize>(), gap);
        assert_eq!(r.gap.is_some(), gap > 0);
        let mut paged = 0;
        let mut cursor: Option<String> = None;
        loop {
            let p = Page::parse(Some(2), cursor.as_deref()).unwrap();
            let page = snap.unresolved(&r.id, &At::Now, p).unwrap();
            assert!(page.rows.iter().all(|u| u.text.chars().count() <= 200));
            paged += page.rows.len();
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(paged, gap, "{}", r.id);
    }
    // The targets: the manifest's where it sets one, else the defaults; none for handlers.
    let targets: Vec<_> = rows.iter().map(|r| r.target).collect();
    assert_eq!(
        targets,
        [
            Some(98.0),
            Some(99.0),
            Some(90.0),
            None,
            Some(80.0),
            Some(75.0)
        ]
    );
    assert_eq!(
        rows[2]
            .gap
            .as_ref()
            .map(|g| (g.what.as_str(), g.why.as_str())),
        Some(("1 call site", "not-found"))
    );

    // Done + unresolved = total on every row at every step, and the derived row is what the
    // intervals give at the last one.
    let last = count("SELECT max(seq) FROM commits");
    for seq in 0..=last {
        for (i, f) in cartograph_store::coverage::figures(&ix.conn, seq, None)
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(f.done + f.unresolved, f.total, "row {i} at {seq}");
        }
    }
    // Golden: the rows and the lists behind them.
    let mut golden = Vec::new();
    for r in &rows {
        golden.push(format!("{} {}/{}", r.id, r.done, r.total));
        for u in snap
            .unresolved(&r.id, &At::Now, Page::default())
            .unwrap()
            .rows
        {
            golden.push(format!("  {}:{} {} ({})", u.path, u.line, u.text, u.reason));
        }
    }
    assert_eq!(
        golden,
        [
            "files 2/3",
            "  web/src/blob.ts:0 web/src/blob.ts (binary)",
            "imports 1/2",
            "  web/src/api.ts:1 ./nope (not-found)",
            "calls 1/2",
            "  web/src/api.ts:6 http://orders:8080/missing (not-found)",
            "handlers 1/2",
            "  orders/openapi.yaml:4 delete /orders/{id} (no-handler)",
            "routes 1/2",
            "  orders/src/server.ts:5 /health (undocumented)",
            "consumers 1/1",
        ]
    );

    // Counts at HEAD are a derived row: a sync with nothing new leaves its key, and rows, alone.
    let key = |c: &cartograph_store::rusqlite::Connection| {
        c.query_row(
            "SELECT inputs_key FROM derived WHERE kind = 'coverage'",
            [],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap()
    };
    let before = key(&ix.conn);
    drop(snap);
    drop((ix, store));
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(s) => assert_eq!(s.steps, 0),
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    assert_eq!(
        key(&Index::open_read(&w.index_path()).unwrap().conn),
        before
    );
}
