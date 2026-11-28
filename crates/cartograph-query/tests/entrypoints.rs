//! XS-20 done-when: the Entrypoints read has the kinds and counts the facts give; a selected HTTP
//! entrypoint lists its consumers with levels (a declared client with the override flag) and a
//! reach summary by service; the trace set is the nodes that summary counts; the count stored at
//! HEAD is the count a walk at HEAD gives; and a past date answers from the links of that date.

use cartograph_api::types::{ConfidenceLevel, EntrypointKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::entrypoints::ClientSpec;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nimport { find } from \"./db\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json(find(1)));\napp.get(\"/health\", (req, res) => res.send(\"ok\"));\n";
const DB: &str =
    "import { pool } from \"./pool\";\nexport function find(id: number) { return pool; }\n";
const POOL: &str = "export const pool = {};\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
const PAY: &str =
    "import { Kafka } from \"kafkajs\";\nproducer.send({ topic: \"order.paid\", messages: [] });\n";
const PAID: &str =
    "import { Kafka } from \"kafkajs\";\nconsumer.subscribe({ topic: \"order.paid\" });\n";
const CRON: &str = "apiVersion: batch/v1\nkind: CronJob\nmetadata:\n  name: nightly-report\nspec:\n  schedule: \"0 2 * * *\"\n";
const PAGE: &str = "export default function Page() { return <main>orders</main>; }\n";
const GO_MAIN: &str = "package main\n\nfunc main() {}\n";

#[test]
fn entrypoints_have_consumers_a_reach_and_a_trace() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write(
        "orders/package.json",
        r#"{"name":"orders","bin":{"orders-cli":"bin/cli.js"},"main":"src/pool.ts"}"#,
    );
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("orders/src/db.ts", DB);
    f.write("orders/src/pool.ts", POOL);
    f.write("orders/bin/cli.js", "console.log('hi');\n");
    f.write("orders/src/pay.ts", PAY);
    f.write("web/src/paid.ts", PAID);
    f.write("web/app/orders/[id]/page.tsx", PAGE);
    f.write("orders/cmd/seed/main.go", GO_MAIN);
    f.write("orders/deploy/cron.yaml", CRON);
    let before = f.commit("orders serves, nothing calls it yet");
    f.write("web/src/api.ts", API);
    f.commit("web calls orders");

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
        clients: vec![ClientSpec {
            name: "Mobile app".into(),
            service: Some("orders".into()),
            calls: vec!["GET /orders/{id}".into()],
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    // Kind counts are the facts': two routes, one subscriber, one CronJob, a bin, a Go main, a
    // main file, a page.
    let all = snap.entrypoints(&At::Now).unwrap();
    let count = |k: EntrypointKind| all.iter().filter(|e| e.kind == k).count();
    assert_eq!(count(EntrypointKind::Http), 2, "{all:#?}");
    assert_eq!(count(EntrypointKind::Event), 1);
    assert_eq!(count(EntrypointKind::Job), 1);
    assert_eq!(count(EntrypointKind::Cli), 2);
    assert_eq!(count(EntrypointKind::Export), 1);
    assert_eq!(count(EntrypointKind::Ui), 1);
    assert_eq!(all.len(), 8);
    assert!(all.iter().any(|e| e.label == "GET /orders/{}"));
    assert!(all.iter().any(|e| e.label == "/orders/:id"));
    assert!(all.iter().any(|e| e.label == "nightly-report (0 2 * * *)"));

    // The HTTP entrypoint: one found caller and one declared client.
    let order = all.iter().find(|e| e.label == "GET /orders/{}").unwrap();
    let health = all.iter().find(|e| e.label == "GET /health").unwrap();
    assert_eq!((order.consumers, health.consumers), (2, 0));
    let d1 = snap.entrypoint(&order.id, &At::Now, None).unwrap();
    assert_eq!(d1.consumers.len(), 2);
    let found = d1.consumers.iter().find(|c| c.node.is_some()).unwrap();
    assert_eq!(
        found.level,
        ConfidenceLevel::Likely,
        "no spec backs the call"
    );
    assert!(!found.overridden);
    let client = d1.consumers.iter().find(|c| c.node.is_none()).unwrap();
    assert_eq!(client.name.as_deref(), Some("Mobile app"));
    assert!(client.overridden);

    // Reach: server -> db -> pool, all in orders; the count stored at HEAD is the walk's.
    assert_eq!(d1.reach.nodes, 2);
    assert_eq!(d1.reach.services.len(), 1);
    assert_eq!(d1.reach.services[0].nodes, 2);
    assert!(!d1.reach.truncated);
    assert_eq!(
        order.reach, d1.reach.nodes,
        "stored HEAD count equals the walk"
    );

    // The trace highlights the same nodes, the entrypoint's own first.
    let t = snap.entrypoint_trace(&order.id, &At::Now, None).unwrap();
    assert_eq!(t.nodes.len() as u32, d1.reach.nodes + 1);
    assert_eq!(t.nodes[0], order.node);

    // Before the web service called it there was no consumer but the declared client.
    let then = snap.entrypoint(&order.id, &At::Sha(before), None).unwrap();
    assert_eq!(then.consumers.len(), 1);
    assert!(snap.entrypoint("nope", &At::Now, None).is_err());
    assert!(
        snap.entrypoint(&order.id, &At::Now, Some("no-such-env"))
            .is_err()
    );
}

/// What a list says without row ids, which two index files needn't share.
fn shape(w: &Workspace, cache: &mut FactCache) -> Vec<(String, String, u32, u32)> {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let mut v: Vec<_> = snap
        .entrypoints(&At::Now)
        .unwrap()
        .into_iter()
        .map(|e| (format!("{:?}", e.kind), e.label, e.consumers, e.reach))
        .collect();
    v.sort();
    v
}

#[test]
fn a_sync_in_two_rounds_equals_one_from_scratch() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write(
        "orders/package.json",
        r#"{"name":"orders","main":"src/pool.ts"}"#,
    );
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("orders/src/db.ts", DB);
    f.write("orders/src/pool.ts", POOL);
    f.commit("orders serves");
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    let ws = |dir: &str| Workspace {
        dir: d.path().join(dir),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("orders"), svc("web")],
        ..Default::default()
    };
    let (a, b) = (ws("a"), ws("b"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    let first = shape(&a, &mut cache);
    assert_eq!(first.len(), 3, "{first:?}");
    f.write("web/src/api.ts", API);
    f.write(
        "orders/src/db.ts",
        "export function find(id: number) { return 1; }\n",
    );
    f.commit("web calls orders, db stops importing pool");
    let two_rounds = shape(&a, &mut cache);
    let scratch = shape(&b, &mut cache);
    assert_eq!(two_rounds, scratch);
    let order = scratch.iter().find(|e| e.1 == "GET /orders/{}").unwrap();
    assert_eq!(
        (order.2, order.3),
        (1, 1),
        "one caller, and db is all it reaches"
    );
}
