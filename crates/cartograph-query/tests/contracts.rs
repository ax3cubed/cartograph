//! XS-08 done-when: the Contracts read gives the four counts a hand count of the fixture gives,
//! and a call that matches a spec operation which its handler's route also matches is exact, with
//! the call site, the route site and the operation as evidence.

use cartograph_api::types::{ConfidenceLevel, Layer, LinkKind, Method, OperationStatus};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\napp.get(\"/health\", (req, res) => res.send(\"ok\"));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
// Three operations: `get` is served and called, `delete` is served by nothing, and `/health` is in
// the code only.
const SPEC: &str = "openapi: 3.0.3
info: {title: Orders API}
paths:
  /orders/{id}:
    get:
      operationId: getOrder
      summary: One order
    delete:
      operationId: deleteOrder
  /orders:
    post:
      operationId: createOrder
";

#[test]
fn the_contracts_read_counts_what_a_hand_count_does() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    let before = f.commit("orders: routes");
    f.write("web/src/api.ts", API);
    f.write("orders/openapi.yaml", SPEC);
    f.commit("web calls, orders documents");

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
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    // Hand count: 3 operations; 1 in sync (get /orders/{id}); 2 spec-only (delete, post /orders);
    // 1 undocumented route (/health).
    let specs = snap.contracts(&At::Now).unwrap();
    assert_eq!(specs.len(), 1);
    let c = &specs[0];
    assert_eq!(
        (c.name.as_str(), c.file.as_str()),
        ("Orders API", "orders/openapi.yaml")
    );
    assert_eq!(
        (
            c.counts.operations,
            c.counts.sync,
            c.counts.spec_only,
            c.counts.undocumented
        ),
        (3, 1, 2, 1)
    );
    let get = c
        .operations
        .iter()
        .find(|o| matches!(o.method, Method::Get))
        .unwrap();
    assert_eq!(get.path, "/orders/{id}");
    assert_eq!(get.status, OperationStatus::Sync);
    assert!(get.handler.is_some());
    assert_eq!(get.callers.len(), 1);
    assert_eq!(get.callers[0].confidence.level, ConfidenceLevel::Exact);
    assert_eq!(
        c.operations
            .iter()
            .filter(|o| o.status == OperationStatus::SpecOnly)
            .count(),
        2
    );
    assert_eq!(c.undocumented.len(), 1);
    assert_eq!(c.undocumented[0].path, "/health");
    // Before the spec existed there was nothing to count.
    assert!(snap.contracts(&At::Sha(before)).unwrap().is_empty());

    // The link: exact, the contract layer, three evidence rows.
    let detail = (1..40)
        .filter_map(|n| snap.link_detail(&format!("l{n}"), &At::Now).ok())
        .find(|d| d.link.kind == LinkKind::Http)
        .expect("the http link");
    assert_eq!(detail.link.confidence.level, ConfidenceLevel::Exact);
    assert!(matches!(detail.layer, Some(Layer::Contract)));
    assert_eq!(detail.evidence.caller.len(), 1);
    assert_eq!(detail.evidence.handler.len(), 1);
    assert_eq!(detail.evidence.contract.len(), 1);
    let op = &detail.evidence.contract[0];
    assert_eq!(
        (op.file.as_str(), op.line, op.kind.as_str()),
        ("orders/openapi.yaml", 4, "contract")
    );
    assert!(
        op.snippet.as_deref().unwrap().contains("/orders/{id}"),
        "{op:?}"
    );
}
