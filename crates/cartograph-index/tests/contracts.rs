//! XS-08: a spec in a service is read into operations, bound to the service's routes and to the
//! calls that consume it, and a call that matches an operation its route also handles is exact.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::contracts::ContractSpec;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\napp.get(\"/health\", (req, res) => res.send(\"ok\"));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
const SPEC: &str = "openapi: 3.0.3
info: {title: Orders}
paths:
  /orders/{id}:
    get:
      operationId: getOrder
      summary: One order
    delete:
      operationId: deleteOrder
components:
  schemas:
    Order:
      properties:
        id: {type: string}
";

fn workspace(f: &Fixture, dir: &Path, globs: &[&str]) -> Workspace {
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
        contracts: globs
            .iter()
            .map(|g| ContractSpec {
                path: (*g).into(),
                service: Some("orders".into()),
            })
            .collect(),
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

/// `kind template op-or-- file:line from-to`, one row per binding.
fn bindings(w: &Workspace) -> Vec<String> {
    let ix = Index::open_read(&w.index_path()).unwrap();
    let mut st = ix
        .conn
        .prepare(
            "SELECT b.kind, b.template, coalesce(o.operation_id, '-'),
                    (SELECT path FROM file_path p WHERE p.file = b.file ORDER BY from_seq DESC LIMIT 1),
                    b.line, b.from_seq, coalesce(CAST(b.to_seq AS TEXT), 'open')
             FROM binding b LEFT JOIN operation o ON o.id = b.operation
             ORDER BY b.kind, b.template, b.from_seq, 4",
        )
        .unwrap();
    st.query_map([], |r| {
        Ok(format!(
            "{} {} {} {}:{} {}-{}",
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn fixture(d: &Path) -> Fixture {
    let mut f = Fixture::init(&d.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.commit("orders: routes");
    f.write("web/src/api.ts", API);
    f.commit("web: fetch an order");
    f.write("orders/openapi.yaml", SPEC);
    f.commit("orders: the spec");
    f
}

#[test]
fn a_spec_is_bound_to_routes_and_calls_and_makes_the_link_exact() {
    let d = tempfile::tempdir().unwrap();
    let f = fixture(d.path());
    let w = workspace(&f, &d.path().join("ws"), &[]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);

    // Hand count at the last commit: two operations; `get /orders/{id}` has a route (sync),
    // `delete` has none (spec-only); `/health` is in a service with a spec and in none.
    assert_eq!(
        bindings(&w),
        [
            "consumes get /orders/{} getOrder web/src/api.ts:2 2-open",
            "handled_by get /orders/{} getOrder orders/src/server.ts:4 2-open",
            "undocumented get /health - orders/src/server.ts:5 2-open",
        ]
    );
    let ix = Index::open_read(&w.index_path()).unwrap();
    let ops: Vec<(String, String, Option<String>, Option<i64>)> = ix
        .conn
        .prepare("SELECT method, path, operation_id, to_seq FROM operation ORDER BY method")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        ops,
        [
            (
                "delete".into(),
                "/orders/{id}".into(),
                Some("deleteOrder".into()),
                None
            ),
            (
                "get".into(),
                "/orders/{id}".into(),
                Some("getOrder".into()),
                None
            ),
        ]
    );
    let schemas: i64 = ix
        .conn
        .query_row("SELECT count(*) FROM contract_schema", [], |r| r.get(0))
        .unwrap();
    assert_eq!(schemas, 1);

    // The call matches the operation, and the operation matches the handler's route: exact, with
    // the call site, the route site and the operation as evidence.
    let (level, score): (String, f64) = ix
        .conn
        .query_row(
            "SELECT level, score FROM link WHERE kind = 'http'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(level, "exact");
    assert!(score >= 0.9, "{score}");
    let mut ev: Vec<(String, String, i64)> = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = e.file LIMIT 1), e.kind, e.start_line
             FROM evidence e JOIN link l ON l.id = e.link WHERE l.kind = 'http'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ev.sort();
    assert_eq!(
        ev,
        [
            ("orders/openapi.yaml".into(), "contract".into(), 4),
            ("orders/src/server.ts".into(), "literal".into(), 4),
            ("web/src/api.ts".into(), "template".into(), 2),
        ]
    );
}

#[test]
fn a_sync_equals_a_rebuild_and_a_second_sync_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fixture(d.path());
    let w = workspace(&f, &d.path().join("ws"), &[]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let first = bindings(&w);
    run(&w, &mut cache);
    assert_eq!(bindings(&w), first);

    // The route is renamed in the code: the operation loses its handler, and the call's link goes.
    f.write(
        "orders/src/server.ts",
        &SERVER.replace("/orders/:id", "/order/:id"),
    );
    f.commit("orders: rename the route");
    run(&w, &mut cache);
    let after = bindings(&w);
    assert!(
        after.contains(&"handled_by get /orders/{} getOrder orders/src/server.ts:4 2-3".to_owned()),
        "{after:?}"
    );
    assert!(
        after.contains(&"undocumented get /order/{} - orders/src/server.ts:4 3-open".to_owned()),
        "{after:?}"
    );

    // A fresh index of the same history says the same.
    let d2 = tempfile::tempdir().unwrap();
    let w2 = workspace(&f, &d2.path().join("ws"), &[]);
    let mut cache2 = FactCache::open(&d2.path().join("facts.db")).unwrap();
    run(&w2, &mut cache2);
    assert_eq!(bindings(&w2), after);
}

#[test]
fn a_manifest_glob_picks_up_a_spec_and_names_its_service() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fixture(d.path());
    // Not named like a spec, and outside the service's folder: only the glob finds it, and the
    // entry says which service owns it.
    f.write("docs/api/v1.yaml", &SPEC.replace("getOrder", "getOrderV1"));
    f.commit("docs: a spec by glob");
    let w = workspace(&f, &d.path().join("ws"), &["docs/api/*.yaml"]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let rows: Vec<(String, String)> = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = c.file LIMIT 1), n.name
             FROM contract c JOIN node n ON n.id = c.service ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            ("docs/api/v1.yaml".to_owned(), "orders".to_owned()),
            ("orders/openapi.yaml".to_owned(), "orders".to_owned()),
        ]
    );
    // Without the glob the file is not read at all.
    let w2 = workspace(&f, &d.path().join("ws2"), &[]);
    let mut cache2 = FactCache::open(&d.path().join("facts2.db")).unwrap();
    run(&w2, &mut cache2);
    let ix = Index::open_read(&w2.index_path()).unwrap();
    let n: i64 = ix
        .conn
        .query_row("SELECT count(*) FROM contract", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn relative_refs_are_read_from_git_and_remote_ones_are_not() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fixture(d.path());
    let root = "openapi: 3.0.3
paths:
  /orders/{id}: {$ref: 'paths/order.yaml'}
  /remote: {$ref: 'https://example.com/x.yaml#/a'}
  /escape: {$ref: '../../outside.yaml'}
";
    f.write("orders/openapi.yaml", root);
    f.write("orders/paths/order.yaml", "get: {operationId: getOrder}\n");
    f.commit("orders: a spec in two files");
    f.write(
        "orders/paths/order.yaml",
        "get: {operationId: getOrder}\nput: {operationId: putOrder}\n",
    );
    f.commit("orders: the path file changes, the spec file does not");

    let w = workspace(&f, &d.path().join("ws"), &[]);
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let ops: Vec<(String, i64, Option<i64>)> = ix
        .conn
        .prepare("SELECT method, from_seq, to_seq FROM operation ORDER BY method")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        ops,
        [
            // The first spec had `delete` until the two-file one replaced it.
            ("delete".to_owned(), 2, Some(3)),
            ("get".to_owned(), 2, None),
            ("put".to_owned(), 4, None),
        ]
    );
    let unresolved: Vec<(String, String)> = ix
        .conn
        .prepare(
            "SELECT spec, reason FROM unresolved WHERE fact_kind = 'openapi.ref' AND to_seq IS NULL
             ORDER BY spec",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        unresolved,
        [
            ("../../outside.yaml".to_owned(), "outside".to_owned()),
            (
                "https://example.com/x.yaml#/a".to_owned(),
                "remote".to_owned()
            ),
        ]
    );
}
