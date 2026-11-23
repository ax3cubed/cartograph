//! HD-04: every GET route in the table, answered over the northwind golden index, validates against
//! the generated OpenAPI file: a 200 against the route's schema, anything else against
//! `ErrorBody`. Generated from the route table, so a route served later is checked without editing
//! this file; a Rust field changed without regenerating the contract fails here.

mod common;

use cartograph_api::routes::{Kind, Response, routes};
use common::*;

fn northwind_data() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let d = tempfile::tempdir().unwrap();
    let data = std::fs::canonicalize(d.path()).unwrap().join("data");
    let ws = data.join("workspaces/northwind");
    std::fs::create_dir_all(&ws).unwrap();
    let built = tempfile::tempdir().unwrap();
    let ix = cartograph_fixtures::index(&cartograph_fixtures::northwind(), built.path());
    std::fs::copy(ix, ws.join("index.cartograph")).unwrap();
    (d, data, "northwind".into())
}

#[test]
fn every_golden_response_validates_against_the_contract() {
    let api = openapi();
    let error = &api["components"]["schemas"]["ErrorBody"];
    let (dir, data, ws) = northwind_data();
    let r = start_in(dir, data, ws);
    let mut ok = 0;
    for route in routes() {
        if route.method != "GET" || route.kind == Kind::Stream {
            continue;
        }
        let path = route
            .path
            .split('/')
            .map(|s| match s {
                "{ws}" => r.ws.clone(),
                s if s.starts_with('{') => "x".into(),
                s => s.into(),
            })
            .collect::<Vec<_>>()
            .join("/");
        let reply = get(&r, &path);
        if reply.status == 200 {
            if !matches!(route.response, Response::Json(_)) {
                continue;
            }
            let schema = &api["paths"][route.path]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"];
            validate(&api, schema, &reply.json(), &path).unwrap_or_else(|e| panic!("{e}"));
            ok += 1;
        } else {
            validate(&api, error, &reply.json(), &path).unwrap_or_else(|e| panic!("{e}"));
        }
    }
    assert!(ok >= 7, "only {ok} routes answered 200 on northwind");
}

/// HD-04 security: every write route's body schema is closed, so unknown input is rejected.
#[test]
fn every_request_body_schema_is_closed() {
    let api = openapi();
    let mut open = Vec::new();
    for (path, ops) in api["paths"].as_object().unwrap() {
        for (method, op) in ops.as_object().unwrap() {
            let Some(s) = op["requestBody"]["content"]["application/json"]["schema"].as_object()
            else {
                continue;
            };
            let s = match s.get("$ref").and_then(|r| r.as_str()) {
                Some(r) => api["components"]["schemas"][r.rsplit('/').next().unwrap()].clone(),
                None => serde_json::Value::Object(s.clone()),
            };
            if s["additionalProperties"] != false {
                open.push(format!("{method} {path}"));
            }
        }
    }
    assert!(
        open.is_empty(),
        "request bodies that accept unknown fields (add #[serde(deny_unknown_fields)] and \
         regenerate the contract): {open:#?}"
    );
}
