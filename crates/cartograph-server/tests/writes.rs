//! SRV-14, this phase's routes: every write is refused without the token, its Origin or a JSON
//! body; bodies are closed schemas; preferences, the folder picker, discovery, adding a workspace
//! (and cancelling its first index) and cartograph.yaml each work end to end.

mod common;

use cartograph_git::fixture::Fixture;
use common::*;

/// The write routes served in this phase, with a valid body.
fn writes(ws: &str) -> Vec<(&'static str, String, Option<&'static str>)> {
    vec![
        ("PUT", "/api/v1/prefs".into(), Some(PREFS)),
        (
            "POST",
            "/api/v1/discover".into(),
            Some(r#"{"path":"~","workspace_file":null}"#),
        ),
        (
            "POST",
            "/api/v1/workspaces".into(),
            Some(r#"{"path":"~/nope","workspace_file":null,"name":null}"#),
        ),
        (
            "POST",
            format!("/api/v1/w/{ws}/jobs"),
            Some(r#"{"kind":"sync","repo":null}"#),
        ),
        ("DELETE", "/api/v1/jobs/0000".into(), None),
        (
            "POST",
            "/api/v1/activity/read".into(),
            Some(r#"{"ids":[]}"#),
        ),
        (
            "PUT",
            format!("/api/v1/w/{ws}/config"),
            Some(r#"{"yaml":""}"#),
        ),
        (
            "POST",
            format!("/api/v1/w/{ws}/config/preview"),
            Some(r#"{"yaml":""}"#),
        ),
    ]
}

const PREFS: &str = r#"{"appearance":{"theme":"light","contrast":"more","motion":"reduce","hues":"safe","size":115,"density":"compact"},"keymap":{"search":"mod+k"},"pins":[],"sidebar_open":false,"ask_placement":"page"}"#;

#[test]
fn every_write_needs_the_token_its_origin_and_json() {
    let r = start();
    let p = r.info.port;
    let host = format!("Host: 127.0.0.1:{p}");
    let (origin, token) = (r.info.origin(), r.info.token.clone());
    for (method, path, body) in writes(&r.ws) {
        let ct = if body.is_some() {
            "\r\nContent-Type: application/json"
        } else {
            ""
        };
        for (head, status, code) in [
            (
                format!("{method} {path} HTTP/1.1\r\n{host}\r\nOrigin: {origin}{ct}"),
                401,
                "unauthorized",
            ),
            (
                format!(
                    "{method} {path} HTTP/1.1\r\n{host}\r\nOrigin: http://evil.example\r\nAuthorization: Bearer {token}{ct}"
                ),
                403,
                "forbidden_origin",
            ),
            (
                format!("{method} {path} HTTP/1.1\r\n{host}\r\nAuthorization: Bearer {token}{ct}"),
                403,
                "forbidden_origin",
            ),
            (
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: evil.example:{p}\r\nOrigin: {origin}\r\nAuthorization: Bearer {token}{ct}"
                ),
                403,
                "forbidden_host",
            ),
        ] {
            let reply = send(p, &head, body);
            assert_eq!(
                (reply.status, reply.code().as_str()),
                (status, code),
                "{head}"
            );
        }
        if let Some(b) = body {
            for ct in [
                "text/plain",
                "application/x-www-form-urlencoded",
                "multipart/form-data",
            ] {
                let reply = send(
                    p,
                    &format!(
                        "{method} {path} HTTP/1.1\r\n{host}\r\nOrigin: {origin}\r\nAuthorization: Bearer {token}\r\nContent-Type: {ct}"
                    ),
                    Some(b),
                );
                assert_eq!(
                    (reply.status, reply.code().as_str()),
                    (415, "unsupported_media_type"),
                    "{method} {path} {ct}"
                );
            }
        }
    }
    assert!(!r.data.join("prefs.json").exists(), "nothing was written");
}

#[test]
fn preferences_round_trip_through_a_closed_schema() {
    let r = start();
    let d = get(&r, "/api/v1/prefs").json();
    assert_eq!(
        d["data"]["appearance"]["theme"], "dark",
        "the app's default"
    );
    let put = write(&r, "PUT", "/api/v1/prefs", Some(PREFS));
    assert_eq!(put.status, 200, "{}", put.body);
    let got = get(&r, "/api/v1/prefs").json();
    assert_eq!(got["data"]["appearance"]["size"], 115);
    assert_eq!(got["data"]["keymap"]["search"], "mod+k");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(r.data.join("prefs.json")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
    }
    let extra = PREFS.replacen('{', r#"{"script":"x","#, 1);
    let bad = write(&r, "PUT", "/api/v1/prefs", Some(&extra));
    assert_eq!((bad.status, bad.code().as_str()), (400, "bad_request"));
    let nested = PREFS.replace(r#""density":"compact""#, r#""density":"compact","x":1"#);
    assert_eq!(write(&r, "PUT", "/api/v1/prefs", Some(&nested)).status, 400);
    let size = PREFS.replace("115", "99");
    assert_eq!(
        write(&r, "PUT", "/api/v1/prefs", Some(&size)).code(),
        "unprocessable"
    );
    // 1 to 64 characters, as the message says: 30 ⌘ keys are 90 bytes but 30 characters.
    let symbols = PREFS.replace("mod+k", &"⌘".repeat(30));
    let p = write(&r, "PUT", "/api/v1/prefs", Some(&symbols));
    assert_eq!(p.status, 200, "{}", p.body);
    let long = PREFS.replace("mod+k", &"⌘".repeat(65));
    assert_eq!(
        write(&r, "PUT", "/api/v1/prefs", Some(&long)).code(),
        "unprocessable"
    );
    let api = openapi();
    let schema = &api["components"]["schemas"]["Prefs"];
    assert_eq!(
        schema["additionalProperties"], false,
        "the published schema is closed too"
    );
}

#[test]
fn the_folder_picker_lists_names_and_flags_inside_home_only() {
    let r = start();
    let root = r.data.parent().unwrap().to_owned();
    std::fs::create_dir_all(root.join("code/.hidden")).unwrap();
    std::fs::write(root.join("code/team.code-workspace"), "{}").unwrap();
    std::fs::write(root.join("secret.txt"), "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc", root.join("etc-link")).unwrap();
    // A folder the server may not read (as Documents is on macOS until the user allows it).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir(root.join("locked")).unwrap();
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
    }
    let list = get(&r, "/api/v1/fs/list?path=~/");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
    }
    assert_eq!(list.status, 200, "{}", list.body);
    let entries = list.json()["data"].as_array().unwrap().clone();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    let mut want = vec!["code", "data", "repo"];
    if cfg!(unix) {
        want.insert(2, "locked");
    }
    assert_eq!(names, want);
    // Nothing inside a child is looked at: `repo` has a .git and `code` a workspace file, yet every
    // flag is unknown until the folder is opened.
    for e in &entries {
        for flag in ["is_repo", "has_workspace_file", "has_children"] {
            assert!(e[flag].is_null(), "{flag} in {e}");
        }
    }
    assert!(!list.body.contains("secret") && !list.body.contains("etc-link"));
    for bad in [
        "/etc",
        "/proc/self",
        "~/../",
        "~/etc-link",
        "relative",
        "%5C%5Cserver%5Cshare",
    ] {
        let reply = get(&r, &format!("/api/v1/fs/list?path={bad}"));
        assert_eq!(reply.status, 400, "{bad}: {}", reply.body);
        validate(
            &openapi(),
            &openapi()["components"]["schemas"]["ErrorBody"],
            &reply.json(),
            bad,
        )
        .unwrap();
    }
}

#[test]
fn discovery_asks_to_pick_one_of_several_workspace_files() {
    let r = start();
    let root = r.data.parent().unwrap().to_owned();
    std::fs::write(
        root.join("a.code-workspace"),
        r#"{"folders":[{"path":"repo"}]}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("b.code-workspace"),
        r#"{"folders":[{"path":"gone"}]}"#,
    )
    .unwrap();
    let many = write(
        &r,
        "POST",
        "/api/v1/discover",
        Some(r#"{"path":"~","workspace_file":null}"#),
    );
    assert_eq!(many.status, 422, "{}", many.body);
    assert!(
        many.json()["error"]["try"]
            .as_str()
            .unwrap()
            .contains("type one")
    );
    let one = write(
        &r,
        "POST",
        "/api/v1/discover",
        Some(r#"{"path":"~","workspace_file":"b.code-workspace"}"#),
    );
    assert_eq!(one.status, 200, "{}", one.body);
    let rows = &one.json()["data"]["folders"];
    assert_eq!(rows[0]["result"], "skipped");
    assert!(rows[0]["reason"].is_string(), "every skipped row says why");
    let a = write(
        &r,
        "POST",
        "/api/v1/discover",
        Some(r#"{"path":"~","workspace_file":"a.code-workspace"}"#),
    );
    assert_eq!(a.json()["data"]["folders"][0]["result"], "repo");
    // IDX-20's rules at the branch tip: a repo with no packages and no deployable is one service.
    let services = &a.json()["data"]["services"];
    assert_eq!(services.as_array().map(Vec::len), Some(1), "{services}");
    assert_eq!(services[0]["detected_by"], "repo");
    assert_eq!(services[0]["library"], false);
    let api = openapi();
    let schema = &api["paths"]["/api/v1/discover"]["post"]["responses"]["200"]["content"]["application/json"]
        ["schema"];
    validate(&api, schema, &a.json(), "discover").unwrap();
}

/// A second repo beside the fixture, for the wizard.
fn new_repo(root: &std::path::Path) -> Fixture {
    let mut f = Fixture::init(&root.join("other"));
    f.write("x.ts", "import './y';\n");
    f.write("y.ts", "");
    f.commit("one");
    f
}

#[test]
fn adding_a_workspace_indexes_it_with_events() {
    let r = start();
    new_repo(r.data.parent().unwrap());
    let mut ev = Events::open(&r);
    let made = write(
        &r,
        "POST",
        "/api/v1/workspaces",
        Some(r#"{"path":"~/other","workspace_file":null,"name":"Other","manifest":true}"#),
    );
    assert_eq!(made.status, 200, "{}", made.body);
    let data = &made.json()["data"];
    let (ws, job) = (
        data["workspace"]["id"].as_str().unwrap(),
        data["job_id"].as_str().unwrap(),
    );
    assert_eq!(data["workspace"]["name"], "Other");
    let started = ev.wait("job.started", 10);
    assert_eq!(started["job_id"], job);
    let progress = ev.wait("job.progress", 30);
    assert_eq!(progress["job_id"], job);
    let finished = ev.wait("job.finished", 60);
    assert_eq!(
        (finished["job_id"].as_str(), finished["ok"].as_bool()),
        (Some(job), Some(true))
    );
    let listed = get(&r, "/api/v1/workspaces").json();
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == ws)
    );
    assert!(
        r.data
            .parent()
            .unwrap()
            .join("other/cartograph.yaml")
            .is_file()
    );
}

#[test]
fn cancelling_a_first_index_leaves_nothing_behind() {
    let r = start();
    new_repo(r.data.parent().unwrap());
    let mut ev = Events::open(&r);
    let made = write(
        &r,
        "POST",
        "/api/v1/workspaces",
        Some(r#"{"path":"~/other","workspace_file":null,"name":null}"#),
    );
    let data = made.json()["data"].clone();
    let (ws, job) = (
        data["workspace"]["id"].as_str().unwrap(),
        data["job_id"].as_str().unwrap(),
    );
    let cancel = write(&r, "DELETE", &format!("/api/v1/jobs/{job}"), None);
    assert_eq!(cancel.status, 200, "{}", cancel.body);
    let finished = ev.wait("job.finished", 60);
    assert_eq!(finished["ok"], false);
    assert!(finished["error"]["what"].is_string());
    let registry = std::fs::read_to_string(r.data.join("registry.toml")).unwrap();
    assert!(!registry.contains(ws), "{registry}");
    assert!(!r.data.join("workspaces").join(ws).exists());
    let gone = get(&r, &format!("/api/v1/workspaces/{ws}"));
    assert_eq!(gone.status, 404, "{}", gone.body);
    // The folder had no manifest: the one this request wrote goes too, so adding it again starts clean.
    let manifest = r.data.parent().unwrap().join("other/cartograph.yaml");
    assert!(!manifest.exists(), "{}", manifest.display());
}

#[test]
fn cancelling_a_first_index_keeps_a_committed_manifest() {
    let r = start();
    new_repo(r.data.parent().unwrap());
    // The folder already has its manifest (committed with the code): it was not this request's.
    let plan =
        cartograph_discovery::setup::plan(&r.data.parent().unwrap().join("other"), None).unwrap();
    let committed = cartograph_discovery::manifest::to_yaml(&plan.manifest);
    std::fs::write(&plan.manifest_path, &committed).unwrap();
    let mut ev = Events::open(&r);
    let made = write(
        &r,
        "POST",
        "/api/v1/workspaces",
        Some(r#"{"path":"~/other","workspace_file":null,"name":null}"#),
    );
    let job = made.json()["data"]["job_id"].as_str().unwrap().to_owned();
    let cancel = write(&r, "DELETE", &format!("/api/v1/jobs/{job}"), None);
    assert_eq!(cancel.status, 200, "{}", cancel.body);
    assert_eq!(ev.wait("job.finished", 60)["ok"], false);
    assert_eq!(
        std::fs::read_to_string(&plan.manifest_path).unwrap(),
        committed
    );
}

#[test]
fn of_two_saves_from_one_version_one_lands_and_the_other_gets_412() {
    let r = start();
    let path = format!("/api/v1/w/{}/config", r.ws);
    let file = r.data.parent().unwrap().join("repo/cartograph.yaml");
    let port = r.info.port;
    let head = |tag: &str| {
        format!(
            "PUT {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {0}\r\nAuthorization: Bearer {1}\r\nContent-Type: application/json\r\nIf-Match: {tag}",
            r.info.origin(),
            r.info.token
        )
    };
    for round in 0..20 {
        let cfg = get(&r, &path).json();
        let tag = cfg["data"]["etag"].as_str().unwrap().to_owned();
        let yaml = cfg["data"]["yaml"].as_str().unwrap().to_owned();
        let gate = std::sync::Barrier::new(2);
        let results: Vec<(u16, String)> = std::thread::scope(|s| {
            let saves: Vec<_> = ["a", "b"]
                .map(|who| {
                    let mine = format!("{yaml}# {round} {who}\n");
                    let body = serde_json::json!({ "yaml": mine }).to_string();
                    let (head, gate) = (head(&tag), &gate);
                    s.spawn(move || {
                        gate.wait();
                        (send(port, &head, Some(&body)).status, mine)
                    })
                })
                .into_iter()
                .collect();
            saves.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut codes: Vec<u16> = results.iter().map(|x| x.0).collect();
        codes.sort_unstable();
        // Never both (a lost update) and never a 500 from a shared temp file.
        assert_eq!(codes, [200, 412], "round {round}");
        let won = &results.iter().find(|x| x.0 == 200).unwrap().1;
        assert_eq!(&std::fs::read_to_string(&file).unwrap(), won);
    }
    let tmp = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(tmp, 0, "no temp file is left behind");
}

#[test]
fn the_manifest_is_read_previewed_and_written_with_if_match() {
    let r = start();
    let cfg = get(&r, &format!("/api/v1/w/{}/config", r.ws));
    assert_eq!(cfg.status, 200, "{}", cfg.body);
    let (etag, yaml) = (
        cfg.json()["data"]["etag"].as_str().unwrap().to_owned(),
        cfg.json()["data"]["yaml"].as_str().unwrap().to_owned(),
    );
    let changed = yaml.replacen("name: repo", "name: renamed", 1);
    assert_ne!(changed, yaml, "{yaml}");
    let body = serde_json::json!({ "yaml": changed }).to_string();
    let preview = write(
        &r,
        "POST",
        &format!("/api/v1/w/{}/config/preview", r.ws),
        Some(&body),
    );
    assert_eq!(preview.status, 200, "{}", preview.body);
    let diff = preview.json()["data"]["diff"].as_str().unwrap().to_owned();
    assert!(
        diff.contains("-  name: repo") || diff.contains("-") && diff.contains("+"),
        "{diff}"
    );
    assert_eq!(preview.json()["data"]["applied"], false);
    let file = r.data.parent().unwrap().join("repo/cartograph.yaml");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        yaml,
        "a preview writes nothing"
    );

    let invalid = serde_json::json!({ "yaml": "version: [" }).to_string();
    let p = write(
        &r,
        "POST",
        &format!("/api/v1/w/{}/config/preview", r.ws),
        Some(&invalid),
    );
    assert_eq!(p.code(), "unprocessable");
    // A name the manifest's own checks allow but sync refuses is caught here, not by every sync after.
    let spaced = yaml.replacen("- name: repo", "- name: My API", 1);
    assert_ne!(spaced, yaml, "{yaml}");
    let body2 = serde_json::json!({ "yaml": spaced }).to_string();
    let p = write(
        &r,
        "POST",
        &format!("/api/v1/w/{}/config/preview", r.ws),
        Some(&body2),
    );
    assert_eq!(p.code(), "unprocessable", "{}", p.body);

    let path = format!("/api/v1/w/{}/config", r.ws);
    let head = |tag: &str| {
        format!(
            "PUT {path} HTTP/1.1\r\nHost: 127.0.0.1:{0}\r\nOrigin: {1}\r\nAuthorization: Bearer {2}\r\nContent-Type: application/json\r\nIf-Match: {tag}",
            r.info.port,
            r.info.origin(),
            r.info.token
        )
    };
    let stale = send(r.info.port, &head("\"0000\""), Some(&body));
    assert_eq!(
        (stale.status, stale.code().as_str()),
        (412, "precondition_failed")
    );
    let mut ev = Events::open(&r);
    let put = send(r.info.port, &head(&etag), Some(&body));
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(put.json()["data"]["applied"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), changed);
    let ev = ev.wait("config.changed", 10);
    assert_eq!(ev["ws"], r.ws.as_str());
    // The old etag no longer matches.
    assert_eq!(send(r.info.port, &head(&etag), Some(&body)).status, 412);
}
