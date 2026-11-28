//! XS-17: a service that reads another service's table directly is a `db` link; the commit that
//! swaps the query for an HTTP call closes it and opens the HTTP link, which says it replaced it.

use cartograph_api::types::{ConfidenceLevel, LinkKind, Lod, NodeKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, GraphQuery, Limits, Store};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const MIGRATION: &str = "CREATE TABLE orders (id int PRIMARY KEY, total int);\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
const ENTITY: &str = "package models\n\ntype Customer struct {\n\tgorm.Model\n}\n";
const DIRECT: &str = "export const one = \"SELECT id, total FROM orders WHERE id = $1\";\n";
const BY_MODEL: &str = "export const all = \"SELECT * FROM customers\";\n";
const BY_NAME: &str = "export const all = \"select * from Orders\";\n";
const OWN: &str = "export const mine = \"SELECT * FROM sessions\";\n";
const VIA_HTTP: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";

/// The repository at its two commits: the direct read, then the HTTP call.
fn history(d: &std::path::Path, first_only: bool) -> (Fixture, String, String) {
    let mut f = Fixture::init(&d.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/db/migrations/001_init.sql", MIGRATION);
    f.write("orders/src/server.ts", SERVER);
    f.write("orders/models/customer.go", ENTITY);
    f.commit("orders: schema and route");
    f.write("web/src/report.ts", DIRECT);
    f.write("web/src/customers.ts", BY_MODEL);
    f.write("web/src/loose.ts", BY_NAME);
    f.write("web/src/own.ts", OWN);
    let direct = f.commit("web: read the orders table");
    if first_only {
        return (f, direct, String::new());
    }
    f.write("web/src/report.ts", VIA_HTTP);
    let swapped = f.commit("web: call the orders API");
    (f, direct, swapped)
}

fn workspace(f: &Fixture, dir: std::path::PathBuf) -> Workspace {
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    Workspace {
        dir,
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("orders"), svc("web")],
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

#[test]
fn a_direct_read_is_a_db_link_and_an_http_call_replaces_it() {
    let d = tempfile::tempdir().unwrap();
    let (f, direct, swapped) = history(d.path(), false);
    let w = workspace(&f, d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    // The service map at the direct read: web reaches orders through a table.
    let service = |at: At| {
        let mut q = GraphQuery::from_params(&Default::default()).unwrap();
        q.lod = Lod::Service;
        q.at = at;
        snap.graph(&q).unwrap()
    };
    let at_direct = service(At::Sha(direct.clone()));
    assert_eq!(at_direct.links.kind, vec![LinkKind::Db]);
    let after = service(At::Sha(swapped.clone()));
    assert!(after.links.kind.contains(&LinkKind::Http));
    assert!(
        !after.links.kind.contains(&LinkKind::Db),
        "own tables stay inside web"
    );

    // Link ids are row ids: try them in turn at each commit.
    let detail = |at: &At, kind: LinkKind| -> Vec<_> {
        (1..40)
            .filter_map(|n| snap.link_detail(&format!("l{n}"), at).ok())
            .filter(|d| d.link.kind == kind)
            .collect()
    };
    let dbs = detail(&At::Sha(direct.clone()), LinkKind::Db);
    // report.ts (exact), customers.ts (the model's derived name: likely), loose.ts (only the name
    // looks alike: guessed) and web's own sessions table (nobody defines it: no link).
    let by_file = |file: &str| {
        dbs.iter()
            .find(|d| d.evidence.caller.first().is_some_and(|c| c.file == file))
            .unwrap_or_else(|| panic!("a db link from {file}"))
    };
    assert_eq!(dbs.len(), 3, "{dbs:#?}");
    let exact = by_file("web/src/report.ts");
    assert_eq!(exact.link.confidence.level, ConfidenceLevel::Exact);
    assert_eq!(
        by_file("web/src/customers.ts").link.confidence.level,
        ConfidenceLevel::Likely
    );
    assert_eq!(
        by_file("web/src/loose.ts").link.confidence.level,
        ConfidenceLevel::Guessed
    );

    // The inspector: operation, the query, the migration that defines the table.
    let op = exact.operation.as_ref().unwrap();
    assert_eq!(
        (op.method.as_str(), op.template.as_str()),
        ("read", "orders")
    );
    let (caller, def) = (&exact.evidence.caller[0], &exact.evidence.handler[0]);
    assert_eq!((caller.line, caller.kind.as_str()), (1, "literal"));
    assert!(caller.snippet.as_deref().unwrap().contains("FROM orders"));
    assert_eq!(def.file, "orders/db/migrations/001_init.sql");
    assert_eq!(exact.opened.sha, direct);
    assert_eq!(exact.closed.as_ref().unwrap().sha, swapped);
    assert!(exact.replaced.is_none());

    // The swap closed the db link and opened the HTTP link, which names it.
    let http = detail(&At::Now, LinkKind::Http);
    assert_eq!(http.len(), 1);
    assert_eq!(http[0].opened.sha, swapped);
    assert_eq!(http[0].replaced.as_ref(), Some(&exact.link.id));
    // The db link's node is the table, owned by orders.
    let table = snap.node_detail(&exact.link.to.0, &At::Now).unwrap().node;
    assert_eq!(
        (table.name.as_str(), table.kind),
        ("orders", NodeKind::Table)
    );
    assert!(table.service.is_some());
}

/// Every link of the index as the inspector shows it, at the last commit.
fn links(w: &Workspace) -> Vec<String> {
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();
    let mut out: Vec<String> = (1..60)
        .filter_map(|n| snap.link_detail(&format!("l{n}"), &At::Now).ok())
        .filter(|d| matches!(d.link.kind, LinkKind::Db | LinkKind::Http))
        .map(|d| {
            format!(
                "{:?} {:?} {} {:?} {:?} {:?}",
                d.link.kind,
                d.link.confidence.level,
                d.evidence.caller.first().map_or("", |c| c.file.as_str()),
                d.opened.message,
                d.closed.map(|c| c.message),
                d.operation.map(|o| o.template),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_sync_of_the_new_commit_equals_a_rebuild() {
    let d = tempfile::tempdir().unwrap();
    let (mut f, _, _) = history(d.path(), true);
    let w = workspace(&f, d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    f.write("web/src/report.ts", VIA_HTTP);
    f.commit("web: call the orders API");
    run(&w, &mut cache);
    let fresh = workspace(&f, d.path().join("fresh"));
    run(
        &fresh,
        &mut FactCache::open(&d.path().join("facts2.db")).unwrap(),
    );
    assert_eq!(links(&w), links(&fresh));
    assert!(links(&w).iter().any(|l| l.starts_with("Db Exact")));
}
