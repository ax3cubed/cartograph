//! XS-19 done-when: on a history with three spec edits (and edits that are not spec changes) the
//! versions list v1 to v3 with the right commit each, the one removal is flagged breaking, and the
//! callers of the removed operation are listed. The full diff is recomputed on request.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";

fn spec(title: &str, get: bool, post: bool, status: &str) -> String {
    let mut s = format!("openapi: 3.0.3\ninfo: {{title: {title}}}\npaths:\n");
    if get {
        s += "  /orders/{id}:\n    get:\n      operationId: getOrder\n";
    }
    if post {
        s += "  /orders:\n    post:\n      operationId: createOrder\n";
    }
    s += &format!(
        "components:\n  schemas:\n    Order:\n      required: [id]\n      properties:\n        id: {{type: string}}\n        status: {{type: string, enum: [{status}]}}\n"
    );
    s
}

#[test]
fn versions_follow_the_spec_hash_and_flag_the_removal() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("web/src/api.ts", API);
    f.write("orders/openapi.yaml", &spec("Orders", true, false, "a, b"));
    let c1 = f.commit("v1: the spec");
    // Not changes: a title, then the file moves.
    f.write(
        "orders/openapi.yaml",
        &spec("Orders API", true, false, "a, b"),
    );
    f.commit("retitle");
    f.remove("orders/openapi.yaml");
    f.write(
        "orders/api/openapi.yaml",
        &spec("Orders API", true, false, "a, b"),
    );
    f.commit("move the spec");
    f.write(
        "orders/api/openapi.yaml",
        &spec("Orders API", true, true, "a, b"),
    );
    let c2 = f.commit("v2: add POST /orders");
    f.write(
        "orders/api/openapi.yaml",
        &spec("Orders API", false, true, "a"),
    );
    let c3 = f.commit("v3: drop GET /orders/{id} and enum value b");

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
    let id = snap.contracts(&At::Now).unwrap()[0].id.clone();

    let v = snap.contract_versions(&id).unwrap();
    assert_eq!(
        v.iter()
            .map(|v| (v.n, v.commit.sha.as_str()))
            .collect::<Vec<_>>(),
        [(1, c1.as_str()), (2, c2.as_str()), (3, c3.as_str())]
    );
    assert_eq!(
        v.iter()
            .map(|v| (v.changes, v.breaking_count))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 0), (2, 2)]
    );
    assert_eq!(v[0].summary, "Initial spec, 1 operations");
    assert_eq!(v[1].summary, "Added POST /orders");
    assert!(v[1].breaking.is_empty());
    // The removal and the enum value are breaking; only the operation has callers.
    let removal = v[2]
        .breaking
        .iter()
        .find(|b| b.kind == "operation_removed")
        .expect("the removal");
    assert_eq!(removal.summary, "Removed GET /orders/{id}");
    assert_eq!(removal.callers.len(), 1);
    assert!(v[2].breaking.iter().any(|b| b.kind == "enum_value_removed"));

    let diff = snap.contract_diff(&id, 1, 3).unwrap();
    assert!(!diff.approximate);
    assert_eq!(diff.changes.iter().filter(|c| c.breaking).count(), 2);
    assert_eq!(diff.changes.len(), 3, "{diff:?}");
    assert!(snap.contract_diff(&id, 1, 9).is_err());
}
