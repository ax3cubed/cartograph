//! XS-09: the link inspector's environment switch. The same call resolves to a different host in
//! each environment and the same service; an unknown environment is a bad request.

use cartograph_api::types::{EvidenceKind, LinkKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, QueryError, Store};

const COMPOSE: &str = "services:
  catalog:
    build: ./catalog
    expose: ['8080']
  web:
    build: ./web
    environment:
      CATALOG_URL: http://catalog:8080
      API_TOKEN: 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
";
const SERVER: &str = "import express from \"express\";\nconst app = express();\napp.get(\"/items/:id\", (req, res) => res.json({}));\n";
const API: &str = "const BASE = process.env.CATALOG_URL;\nexport const load = (id: string) => fetch(`${BASE}/items/${id}`);\n";
const SERVICE: &str = "kind: Service\nmetadata: {name: catalog, namespace: prod}\nspec:\n  ports:\n    - port: 8080\n";
const TEMPLATE: &str = "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n          env:\n            - name: CATALOG_URL\n              value: {{ .Values.catalog.url | quote }}\n";
const VALUES: &str = "catalog:\n  url: http://catalog.prod.svc:8080\n";

#[test]
fn switching_the_environment_changes_the_host_and_not_the_service() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("catalog/package.json", r#"{"name":"catalog"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("catalog/src/server.ts", SERVER);
    f.write("catalog/k8s/service.yaml", SERVICE);
    f.write("web/src/api.ts", API);
    f.write("web/chart/templates/deployment.yaml", TEMPLATE);
    f.write("web/chart/values-prod.yaml", VALUES);
    f.commit("everything");
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
        services: vec![svc("catalog"), svc("web")],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    // The environments, prod the default, with the locators each has and no secret.
    let envs = snap.environments().unwrap();
    let names: Vec<_> = envs
        .iter()
        .map(|e| (e.name.as_str(), e.is_default))
        .collect();
    assert_eq!(names, [("local", false), ("prod", true)]);
    assert_eq!(envs[0].values["CATALOG_URL"], "http://catalog:8080");
    assert_eq!(
        envs[1].values["CATALOG_URL"],
        "http://catalog.prod.svc:8080"
    );
    assert!(!envs.iter().any(|e| e.values.contains_key("API_TOKEN")));

    let detail = |env: Option<&str>| {
        (1..40)
            .filter_map(|n| snap.link_detail_in(&format!("l{n}"), &At::Now, env).ok())
            .find(|d| d.link.kind == LinkKind::Http)
            .expect("an http link")
    };
    // Without `env`: the default, prod.
    let prod = detail(None);
    assert_eq!(prod.env.as_deref(), Some("prod"));
    assert_eq!(prod.envs, ["local", "prod"]);
    assert_eq!(prod.config.len(), 1);
    assert_eq!(prod.config[0].key, "CATALOG_URL");
    assert_eq!(prod.config[0].host, "catalog.prod.svc:8080");
    assert_eq!(prod.config[0].source, "web/chart/values-prod.yaml");
    assert!(
        prod.link
            .confidence
            .evidence
            .iter()
            .any(|e| e.kind == EvidenceKind::Env && e.note.contains("catalog.prod.svc"))
    );

    let local = detail(Some("local"));
    assert_eq!(local.env.as_deref(), Some("local"));
    assert_eq!(local.config[0].host, "catalog:8080");
    assert_eq!(local.config[0].source, "docker-compose.yml");
    // The same link: the same two services either way.
    assert_eq!(local.link.id, prod.link.id);
    assert_eq!(
        (&local.link.from, &local.link.to),
        (&prod.link.from, &prod.link.to)
    );

    match snap.link_detail_in(&prod.link.id.0, &At::Now, Some("nope")) {
        Err(QueryError::BadRequest { .. }) => {}
        other => panic!("{other:?}"),
    }
}
