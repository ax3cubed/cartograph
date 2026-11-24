//! XS-06: the detail of the HTTP link of the XS-04 fixture (`fetch("http://orders:8080/orders/" +
//! id)` calling `GET /orders/:id`): its level, evidence with snippets, the operation, the commits
//! that opened and closed it, and the link it replaced.

use std::path::Path;

use cartograph_api::types::{ConfidenceLevel, LinkKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, GraphQuery, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
const MOVED: &str = "import express from \"express\";\nconst app = express();\nconst token = \"AKIAIOSFODNN7EXAMPLE\";\n\napp.get(\"/orders/:id\", (req, res) => res.json({ moved: true }));\napp.get(\"/health\", (req, res) => res.send(\"ok\"));\nexport default app;\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";

fn workspace(f: &Fixture, dir: &Path) -> Workspace {
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
        ..Default::default()
    }
}

#[test]
fn a_link_answers_with_its_evidence_and_its_commits() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.commit("orders: the route");
    f.write("web/src/api.ts", API);
    let opened = f.commit("web: fetch an order");
    // The route moves to another file (new content, so not a rename): the link follows it.
    f.git(&["rm", "-q", "orders/src/server.ts"]);
    f.write("orders/src/routes.ts", MOVED);
    let moved = f.commit("orders: move the route");

    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    // The map's payload carries kind, level and score for the HTTP link.
    let slice = snap
        .graph(&GraphQuery::from_params(&Default::default()).unwrap())
        .unwrap();
    let i = slice
        .links
        .kind
        .iter()
        .position(|k| *k == LinkKind::Http)
        .expect("an http link on the map");
    assert_eq!(slice.links.level[i], ConfidenceLevel::Likely);
    assert!(!slice.links.corrected[i]);

    // Link ids are row ids: try them in turn, at the commit that opened the first link.
    let http_at = |at: &At| -> Vec<_> {
        (1..40)
            .filter_map(|n| snap.link_detail(&format!("l{n}"), at).ok())
            .filter(|d| d.link.kind == LinkKind::Http)
            .collect()
    };
    // (A link not yet open at `at` answers with its first interval, so pick the one opened there.)
    let seen: Vec<_> = http_at(&At::Sha(opened.clone()))
        .into_iter()
        .filter(|d| d.opened.sha == opened)
        .collect();
    assert_eq!(seen.len(), 1, "one http link opens at the second commit");
    let first = &seen[0];
    // The spec says Exact; a template call site can't reach it without a contract (XS-08).
    assert_eq!(first.link.confidence.level, ConfidenceLevel::Likely);
    let op = first.operation.as_ref().unwrap();
    assert_eq!(
        (op.method.as_str(), op.template.as_str()),
        ("get", "/orders/{}")
    );
    assert!(first.layer.is_none());
    assert!(first.evidence.contract.is_empty());
    let (caller, handler) = (&first.evidence.caller[0], &first.evidence.handler[0]);
    assert_eq!(
        (caller.file.as_str(), caller.line, caller.kind.as_str()),
        ("web/src/api.ts", 2, "template")
    );
    assert!(
        caller.snippet.as_deref().unwrap().contains("fetch("),
        "{caller:?}"
    );
    assert_eq!(
        (handler.file.as_str(), handler.line),
        ("orders/src/server.ts", 4)
    );
    assert!(handler.snippet.as_deref().unwrap().contains("app.get("));
    assert_eq!(first.opened.sha, opened);
    assert_eq!(first.opened.message, "web: fetch an order");
    assert!(!first.opened.author.is_empty());
    assert_eq!(first.closed.as_ref().unwrap().sha, moved);
    assert!(first.replaced.is_none());

    // The new link opened where the old one closed, between the same services.
    let now = http_at(&At::Now)
        .into_iter()
        .find(|d| d.closed.is_none())
        .expect("the link to the moved route");
    assert_eq!(now.replaced.as_ref(), Some(&first.link.id));
    let h = &now.evidence.handler[0];
    assert_eq!(h.file, "orders/src/routes.ts");
    // Secrets in the snippet are masked.
    let text = h.snippet.as_deref().unwrap();
    assert!(!text.contains("AKIAIOSFODNN7EXAMPLE"), "{text}");
}
