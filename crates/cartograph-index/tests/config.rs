//! XS-09: a call through `CATALOG_URL` links to the same service from two hosts, one per
//! environment; removing the Helm value closes the prod side at that commit; a sync equals a
//! rebuild; no secret value reaches the index.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::config::EnvSpec;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const COMPOSE: &str = "services:
  catalog:
    build: ./catalog
    expose: ['8080']
  web:
    build: ./web
    environment:
      CATALOG_URL: http://catalog:8080
      DB_PASSWORD: hunter2hunter2
";
const SERVER: &str = "import express from \"express\";\nconst app = express();\napp.get(\"/items/:id\", (req, res) => res.json({}));\n";
const API: &str = "const BASE = process.env.CATALOG_URL;\nexport const load = (id: string) => fetch(`${BASE}/items/${id}`);\n";
const SERVICE: &str = "kind: Service\nmetadata: {name: catalog, namespace: prod}\nspec:\n  ports:\n    - port: 8080\n";
const TEMPLATE: &str = "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n          env:\n            - name: CATALOG_URL\n              value: {{ .Values.catalog.url | quote }}\n";
const VALUES: &str = "catalog:\n  url: http://catalog.prod.svc:8080\n";

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
        services: vec![svc("catalog"), svc("web")],
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn q<T: std::fmt::Debug>(
    ix: &Index,
    sql: &str,
    f: impl Fn(&cartograph_store::rusqlite::Row) -> T,
) -> Vec<T> {
    ix.conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| Ok(f(r)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// `env host-of-value source from-to` of what the one link was resolved through.
fn chain(ix: &Index) -> Vec<String> {
    q(
        ix,
        "SELECT e.name, c.value, c.source, c.from_seq, coalesce(CAST(c.to_seq AS TEXT), 'open')
         FROM link_config c JOIN environment e ON e.id = c.env_id ORDER BY e.name, c.from_seq",
        |r| {
            format!(
                "{} {} {} {}-{}",
                r.get::<_, String>(0).unwrap(),
                r.get::<_, String>(1).unwrap(),
                r.get::<_, String>(2).unwrap(),
                r.get::<_, i64>(3).unwrap(),
                r.get::<_, String>(4).unwrap()
            )
        },
    )
}

fn fixture(d: &Path) -> Fixture {
    let mut f = Fixture::init(&d.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("catalog/package.json", r#"{"name":"catalog"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("catalog/src/server.ts", SERVER);
    f.write("catalog/k8s/service.yaml", SERVICE);
    f.commit("catalog: the route and its Service");
    f.write("web/src/api.ts", API);
    f.commit("web: load an item through CATALOG_URL");
    f.write("web/chart/templates/deployment.yaml", TEMPLATE);
    f.write("web/chart/values-prod.yaml", VALUES);
    f.commit("web: the chart gives prod its host");
    f
}

#[test]
fn one_call_two_hosts_one_service_and_the_helm_value_closes_prod() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fixture(d.path());
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();

    // Environments: local from compose, prod from the chart's values file; prod is the default.
    assert_eq!(
        q(
            &ix,
            "SELECT id, name, is_default FROM environment ORDER BY id",
            |r| (
                r.get::<_, i64>(0).unwrap(),
                r.get::<_, String>(1).unwrap(),
                r.get::<_, i64>(2).unwrap()
            )
        ),
        [(0, "local".into(), 0), (1, "prod".into(), 1)]
    );
    // One link, web to catalog, likely, holding in both environments, with a host for each.
    assert_eq!(
        q(
            &ix,
            "SELECT level, envs FROM link WHERE kind = 'http'",
            |r| (r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap())
        ),
        [("likely".into(), 3)]
    );
    assert_eq!(
        chain(&ix),
        [
            "local http://catalog:8080 docker-compose.yml 1-open",
            "prod http://catalog.prod.svc:8080 web/chart/values-prod.yaml 2-open"
        ]
    );
    // No secret value is stored anywhere.
    let planted = q(
        &ix,
        "SELECT count(*) FROM config_value WHERE value LIKE '%hunter2%'",
        |r| r.get::<_, i64>(0).unwrap(),
    );
    assert_eq!(planted, [0]);
    assert_eq!(
        q(
            &ix,
            "SELECT value_kind, value FROM config_value WHERE key = 'DB_PASSWORD'",
            |r| (
                r.get::<_, String>(0).unwrap(),
                r.get::<_, Option<String>>(1).unwrap()
            )
        ),
        [("opaque".into(), None)]
    );

    // Removing the Helm value closes prod at that commit; local still holds the link.
    f.write("web/chart/values-prod.yaml", "other: 1\n");
    f.commit("web: drop prod's catalog url");
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        chain(&ix),
        [
            "local http://catalog:8080 docker-compose.yml 1-open",
            "prod http://catalog.prod.svc:8080 web/chart/values-prod.yaml 2-3"
        ]
    );
    // The call has no value in prod from then on.
    assert_eq!(
        q(
            &ix,
            "SELECT reason, from_seq, to_seq FROM unresolved WHERE fact_kind = 'http.call'",
            |r| (
                r.get::<_, String>(0).unwrap(),
                r.get::<_, i64>(1).unwrap(),
                r.get::<_, Option<i64>>(2).unwrap()
            )
        ),
        [
            ("no value for CATALOG_URL in prod".to_owned(), 1, Some(2)),
            ("no value for CATALOG_URL in prod".to_owned(), 3, None)
        ]
    );

    // Syncing commit by commit equals building at once.
    let w_once = workspace(&f, &d.path().join("once"));
    run(&w_once, &mut cache);
    let once = Index::open_read(&w_once.index_path()).unwrap();
    assert_eq!(chain(&once), chain(&ix));
}

/// With only the chart giving a value, removing it closes the link itself at that commit.
#[test]
fn removing_the_only_value_closes_the_link() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write(
        "docker-compose.yml",
        "services:\n  catalog:\n    build: ./catalog\n  web:\n    build: ./web\n",
    );
    f.write("catalog/package.json", r#"{"name":"catalog"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("catalog/src/server.ts", SERVER);
    f.write("catalog/k8s/service.yaml", SERVICE);
    f.write("web/src/api.ts", API);
    f.write("web/chart/templates/deployment.yaml", TEMPLATE);
    f.write("web/chart/values-prod.yaml", VALUES);
    f.commit("everything");
    f.write("note.txt", "x");
    f.commit("unrelated");
    f.write("web/chart/values-prod.yaml", "other: 1\n");
    f.commit("drop the value");
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        q(
            &ix,
            "SELECT i.from_seq, i.to_seq FROM interval i JOIN link l ON l.id = i.link
             WHERE l.kind = 'http'",
            |r| (
                r.get::<_, i64>(0).unwrap(),
                r.get::<_, Option<i64>>(1).unwrap()
            )
        ),
        [(0, Some(2))]
    );
}

/// Values set in the manifest hold for the whole history and are classified like any other.
#[test]
fn manifest_values_resolve_a_call_and_keep_no_secret() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write(
        "docker-compose.yml",
        "services:\n  catalog:\n    build: ./catalog\n  web:\n    build: ./web\n",
    );
    f.write("catalog/package.json", r#"{"name":"catalog"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("catalog/src/server.ts", SERVER);
    f.write("web/src/api.ts", API);
    f.commit("everything");
    let mut w = workspace(&f, &d.path().join("ws"));
    w.environments = vec![EnvSpec {
        name: "stage".into(),
        sources: vec![],
        set: [
            ("CATALOG_URL".to_owned(), "http://catalog:8080".to_owned()),
            (
                "API_TOKEN".to_owned(),
                "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".to_owned(),
            ),
        ]
        .into(),
    }];
    w.default_environment = Some("stage".into());
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(
        chain(&ix),
        ["stage http://catalog:8080 cartograph.yaml 0-open"]
    );
    assert_eq!(
        q(
            &ix,
            "SELECT count(*) FROM config_value WHERE value LIKE '%ghp_%'",
            |r| r.get::<_, i64>(0).unwrap()
        ),
        [0]
    );
}
