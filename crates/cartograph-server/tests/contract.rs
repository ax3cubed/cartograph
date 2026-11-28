//! SRV-07's done-when: every route's body validates against the generated schema, and so does
//! every error body, whatever produced it.

mod common;

use common::*;

/// Every route the server serves, with the table row it implements.
const SERVED: &[(&str, &str)] = &[
    ("/api/version", "/api/version"),
    ("/api/v1/meta", "/api/v1/meta"),
    ("/api/v1/workspaces", "/api/v1/workspaces"),
    ("/api/v1/workspaces/{ws}", "/api/v1/workspaces/{ws}"),
    ("/api/v1/w/{ws}/freshness", "/api/v1/w/{ws}/freshness"),
    ("/api/v1/w/{ws}/repos", "/api/v1/w/{ws}/repos"),
    ("/api/v1/w/{ws}/graph", "/api/v1/w/{ws}/graph"),
    (
        "/api/v1/w/{ws}/graph?at=now&lod=file&kinds=imports",
        "/api/v1/w/{ws}/graph",
    ),
    ("/api/v1/w/{ws}/graph?lod=module", "/api/v1/w/{ws}/graph"),
    ("/api/v1/w/{ws}/graph?lod=service", "/api/v1/w/{ws}/graph"),
    ("/api/v1/activity", "/api/v1/activity"),
    // SRV-12
    ("/api/v1/w/{ws}/entrypoints", "/api/v1/w/{ws}/entrypoints"),
    (
        "/api/v1/w/{ws}/entrypoints?kind=http&q=a&sort=-reach&limit=5",
        "/api/v1/w/{ws}/entrypoints",
    ),
    (
        "/api/v1/w/{ws}/entrypoints/counts",
        "/api/v1/w/{ws}/entrypoints/counts",
    ),
    ("/api/v1/w/{ws}/concepts", "/api/v1/w/{ws}/concepts"),
    ("/api/v1/w/{ws}/owners", "/api/v1/w/{ws}/owners"),
    ("/api/v1/w/{ws}/coverage", "/api/v1/w/{ws}/coverage"),
    (
        "/api/v1/w/{ws}/coverage/imports/unresolved?limit=10",
        "/api/v1/w/{ws}/coverage/{row}/unresolved",
    ),
];

#[test]
fn every_route_body_validates_against_the_generated_schema() {
    let api = openapi();
    let table = cartograph_api::routes::routes();
    let r = start();
    for (path, row) in SERVED {
        assert!(
            table.iter().any(|t| t.method == "GET" && t.path == *row),
            "{row} isn't in the route table"
        );
        let reply = get(&r, &path.replace("{ws}", &r.ws));
        assert_eq!(reply.status, 200, "{path}: {}", reply.body);
        assert_eq!(reply.headers["content-type"], "application/json", "{path}");
        let schema =
            &api["paths"][row]["get"]["responses"]["200"]["content"]["application/json"]["schema"];
        assert!(schema.is_object(), "{row} has no 200 schema");
        validate(&api, schema, &reply.json(), path).unwrap_or_else(|e| panic!("{e}"));
        if row.contains("/w/{ws}/") {
            let generation = reply.json()["generation"].as_u64().unwrap();
            assert_eq!(
                reply.headers["cartograph-generation"],
                generation.to_string()
            );
            assert_eq!(reply.headers["cartograph-state"], "current");
        }
    }
    let slice = get(&r, &format!("/api/v1/w/{}/graph", r.ws)).json();
    assert_eq!(slice["data"]["nodes"]["name"].as_array().unwrap().len(), 4);
    assert_eq!(slice["data"]["links"]["from"].as_array().unwrap().len(), 3);
}

/// SRV-12: there is no concept to show yet, a bad filter is a 400, and a commit's impact is text
/// ending in the freshness line, the way `cartograph impact` prints it.
#[test]
fn the_view_reads_refuse_bad_input_and_answer_in_text_where_they_say_so() {
    let r = start();
    let w = |tail: &str| get(&r, &format!("/api/v1/w/{}/{tail}", r.ws));
    // The reads that come from the manifest, not a snapshot, carry no generation header.
    let api = openapi();
    for (tail, row) in [
        ("adapters", "adapters"),
        ("rules", "rules"),
        ("rules/violations?limit=10", "rules/violations"),
        ("rules/commits?limit=10", "rules/commits"),
    ] {
        let reply = w(tail);
        assert_eq!(reply.status, 200, "{tail}: {}", reply.body);
        let schema = &api["paths"][format!("/api/v1/w/{{ws}}/{row}")]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];
        validate(&api, schema, &reply.json(), tail).unwrap_or_else(|e| panic!("{e}"));
    }
    let built_in = w("adapters").json()["data"].as_array().unwrap().len();
    assert!(built_in > 0, "the built-in adapters are listed");
    assert_eq!(w("concepts/invoice").status, 404);
    assert_eq!(w("entrypoints?kind=nope").status, 400);
    assert_eq!(w("entrypoints?sort=sideways").status, 400);
    assert_eq!(w("entrypoints?cursor=x").status, 400);
    assert_eq!(w("rules/violations?limit=0").status, 400);
    let sha = w("commits").json()["data"][0]["sha"]
        .as_str()
        .unwrap()
        .to_owned();
    let md = w(&format!("commits/{sha}/impact/markdown"));
    assert_eq!(md.status, 200, "{}", md.body);
    assert!(md.headers["content-type"].starts_with("text/plain"));
    assert!(md.body.contains("affected in"), "{}", md.body);
    assert!(md.body.trim_end().ends_with('_'), "{}", md.body);
    assert_eq!(w("commits/ffffffffff/impact/markdown").status, 404);
}

/// XS-06: a link's id from the map answers `…/links/{id}` with both commits and evidence, and the
/// body validates against the contract. (HTTP evidence is covered over the XS-04 fixture in
/// `cartograph-query`'s `link_detail` test.)
#[test]
fn a_link_from_the_map_answers_with_its_detail() {
    let api = openapi();
    let r = start();
    let slice = get(&r, &format!("/api/v1/w/{}/graph", r.ws)).json();
    let id = slice["data"]["links"]["id"][0].as_str().unwrap().to_owned();
    let path = format!("/api/v1/w/{}/links/{id}", r.ws);
    let reply = get(&r, &path);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let schema = &api["paths"]["/api/v1/w/{ws}/links/{id}"]["get"]["responses"]["200"]["content"]["application/json"]
        ["schema"];
    let body = reply.json();
    validate(&api, schema, &body, &path).unwrap_or_else(|e| panic!("{e}"));
    let d = &body["data"];
    assert_eq!(d["link"]["id"], id.as_str());
    assert_eq!(d["layer"], "base");
    assert!(d["opened"]["author"].is_string());
    assert!(d["closed"].is_null());
    assert!(d["evidence"]["caller"].as_array().unwrap().is_empty());
    assert_eq!(
        get(&r, &format!("/api/v1/w/{}/links/l99999", r.ws)).status,
        404
    );
}

#[test]
fn every_error_body_validates_and_leaks_nothing() {
    let api = openapi();
    let error = &api["components"]["schemas"]["ErrorBody"];
    let r = start();
    let p = r.info.port;
    let ws = r.ws.clone();
    let cases: Vec<(Reply, u16, &str)> = vec![
        (get(&r, "/api/v1/nope"), 404, "not_found"),
        (get(&r, "/api/v1/w/nope/graph"), 404, "not_found"),
        (get(&r, "/api/v1/w/..%2f..%2fetc/graph"), 404, "not_found"),
        (
            get(&r, &format!("/api/v1/w/{ws}/graph?lod=workspace")),
            400,
            "bad_request",
        ),
        (
            get(&r, &format!("/api/v1/w/{ws}/graph?lod=galaxy")),
            400,
            "bad_request",
        ),
        (
            get(&r, &format!("/api/v1/w/{ws}/graph?at=yesterday")),
            400,
            "bad_request",
        ),
        (
            get(&r, &format!("/api/v1/w/{ws}/graph?at=0000000000")),
            404,
            "not_found",
        ),
        (
            get(
                &r,
                &format!("/api/v1/w/{ws}/graph?at=%27%3B%20DROP%20TABLE%20link%3B--"),
            ),
            400,
            "bad_request",
        ),
        (
            raw(
                p,
                &format!("GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}"),
            ),
            401,
            "unauthorized",
        ),
        (
            raw(p, "GET /api/v1/meta HTTP/1.1\r\nHost: evil.example"),
            403,
            "forbidden_host",
        ),
        (
            raw(
                p,
                &format!(
                    "POST /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: 0",
                    r.info.origin(),
                    r.info.token
                ),
            ),
            405,
            "method_not_allowed",
        ),
    ];
    let home = r.data.parent().unwrap().to_string_lossy().into_owned();
    for (reply, status, code) in &cases {
        assert_eq!(
            (reply.status, reply.code().as_str()),
            (*status, *code),
            "{}",
            reply.body
        );
        validate(&api, error, &reply.json(), code).unwrap_or_else(|e| panic!("{e}"));
        for leak in [
            "SELECT",
            "FROM ",
            "sqlite",
            "SQLITE",
            "panicked",
            "RUST_BACKTRACE",
            home.as_str(),
        ] {
            assert!(!reply.body.contains(leak), "{leak} in {}", reply.body);
        }
        assert!(reply.json().get("data").is_none());
    }
}

#[test]
fn a_workspace_is_listed_by_its_own_name_and_root_folder() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let acme = root.join("acme");
    for n in ["api", "web"] {
        let mut f = cartograph_git::fixture::Fixture::init(&acme.join(n));
        f.write("src/a.ts", "export const a = 1;\n");
        f.commit("one");
    }
    std::fs::write(
        acme.join("acme.code-workspace"),
        r#"{ "folders": [ { "path": "api" }, { "path": "web" } ] }"#,
    )
    .unwrap();
    let data = root.join("data");
    let ws = register(&data, &acme);
    let r = start_in(d, data, ws.clone());
    // Not its first repo's name and folder ("api", acme/api): the workspace's own, from the registry.
    let list = get(&r, "/api/v1/workspaces").json();
    let w = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == ws.as_str())
        .cloned()
        .unwrap();
    assert_eq!(w["name"], "acme", "{w}");
    assert_eq!(w["path"], acme.display().to_string(), "{w}");
    assert_eq!(w["repos"], 2);
    let one = get(&r, &format!("/api/v1/workspaces/{ws}")).json();
    assert_eq!(one["data"]["name"], "acme", "{one}");
}
