//! XS-12: a link a correction decided answers with `layer = correction`, the entry's id and the
//! values the passes found; the map marks it `corrected`.

use cartograph_api::types::{ConfidenceLevel, Layer, LinkKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, GraphQuery, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
const SET: &str = "version: 1
corrections:
  - action: set
    key: {kind: http, from: 'web:web/src/api.ts', to: 'orders:orders/src/server.ts', op: 'GET /orders/{id}'}
    confidence: exact
    reason: the contract says so
";

#[test]
fn a_corrected_link_names_its_layer_and_its_base() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("web/src/api.ts", API);
    f.write("cartograph.overrides.yaml", SET);
    f.commit("all of it");
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
        overrides: Some(f.dir.join("cartograph.overrides.yaml")),
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let slice = snap
        .graph(&GraphQuery::from_params(&Default::default()).unwrap())
        .unwrap();
    let i = slice
        .links
        .kind
        .iter()
        .position(|k| *k == LinkKind::Http)
        .unwrap();
    assert_eq!(slice.links.level[i], ConfidenceLevel::Exact);
    assert!(slice.links.corrected[i]);

    let id = format!(
        "l{}",
        (1..40)
            .find(|n| snap
                .link_detail(&format!("l{n}"), &At::Now)
                .is_ok_and(|d| d.link.kind == LinkKind::Http))
            .unwrap()
    );
    let d = snap.link_detail(&id, &At::Now).unwrap();
    assert!(matches!(d.layer, Some(Layer::Correction)));
    // `c<position>-<fingerprint>`: the API's id of the entry.
    let id = d.correction_id.as_deref().unwrap();
    assert!(id.starts_with("c0-") && id.len() == 11, "{id}");
    assert_eq!(d.link.confidence.level, ConfidenceLevel::Exact);
    let base = d.base.expect("the passes' values are kept");
    assert_eq!((base.level, base.score), (ConfidenceLevel::Likely, 0.89));
    assert!(base.to.is_none());
}
