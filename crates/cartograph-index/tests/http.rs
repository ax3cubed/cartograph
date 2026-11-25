//! XS-04: a fetch of `http://orders:8080/orders/` + id links to `GET /orders/:id` in `orders`, from
//! the commit that opens it to the one that deletes the route, and a sync equals a rebuild.

use std::collections::BTreeSet;
use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
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

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

/// `caller -> route from-to` per HTTP interval.
fn links(w: &Workspace) -> Vec<String> {
    let ix = Index::open_read(&w.index_path()).unwrap();
    let mut st = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = ns.file ORDER BY from_seq DESC LIMIT 1),
                    (SELECT path FROM file_path p WHERE p.file = nd.file ORDER BY from_seq DESC LIMIT 1),
                    i.from_seq, coalesce(CAST(i.to_seq AS TEXT), 'open')
             FROM interval i JOIN link l ON l.id = i.link
               JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
             WHERE l.kind = 'http' ORDER BY 1, 2, i.from_seq",
        )
        .unwrap();
    st.query_map([], |r| {
        Ok(format!(
            "{} -> {} {}-{}",
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn a_fetch_links_to_the_route_until_the_route_is_deleted() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.commit("orders: the route");
    f.write("web/src/api.ts", API);
    let opened = f.commit("web: fetch an order");
    f.write("web/src/other.ts", "export const x = 1;\n");
    f.commit("unrelated");
    f.git(&["rm", "-q", "orders/src/server.ts"]);
    f.commit("orders: drop the route");

    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    assert_eq!(links(&w), ["web/src/api.ts -> orders/src/server.ts 1-3"]);

    let ix = Index::open_read(&w.index_path()).unwrap();
    // The commit that opened it.
    let sha: Vec<u8> = ix
        .conn
        .query_row("SELECT sha FROM commits WHERE seq = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hex(&sha), opened);
    // Its evidence: the call site and the route site, with lines.
    let mut ev: Vec<(String, String, i64, String)> = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = e.file LIMIT 1), e.kind,
                    e.start_line, e.extractor FROM evidence e
             JOIN link l ON l.id = e.link WHERE l.kind = 'http'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ev.sort();
    assert_eq!(
        ev,
        [
            (
                "orders/src/server.ts".into(),
                "literal".into(),
                4,
                "express@1.0.0".into()
            ),
            (
                "web/src/api.ts".into(),
                "template".into(),
                2,
                "fetch@1.0.0".into()
            ),
        ]
    );
    // Its confidence: a template call site (likely), an alias-resolved host (likely) and a full
    // path match anchored on the express handler. No step is exact on its own, so the link is
    // likely; the noisy-or of the three likely rows is over the band, so it takes the band's top.
    assert_eq!(level_score(&ix), [("likely".to_owned(), 0.89)]);
    // While the route is gone the call is not-found in orders, from the commit that dropped it.
    let u: Vec<(String, String, i64, Option<i64>)> = ix
        .conn
        .prepare(
            "SELECT spec, reason, from_seq, to_seq FROM unresolved WHERE fact_kind = 'http.call'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        u,
        [("get /orders/{}".to_owned(), "not-found".to_owned(), 3, None)]
    );
}

fn level_score(ix: &Index) -> Vec<(String, f64)> {
    ix.conn
        .prepare("SELECT level, score FROM link WHERE kind = 'http'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// With no host the only route that fits is the base evidence, which is guessed: the call site
/// is a literal and the path a full match on a handler, but the weakest step sets the level.
#[test]
fn a_hostless_call_with_one_candidate_is_guessed() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write(
        "web/src/api.ts",
        "export const load = () => fetch(\"/orders/42\");\n",
    );
    f.commit("both");
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    assert_eq!(links(&w), ["web/src/api.ts -> orders/src/server.ts 0-open"]);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(level_score(&ix), [("guessed".to_owned(), 0.40)]);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Twelve commits adding, editing and removing routes and calls: the links live at every commit
/// are the ones the files present then say, and syncing commit by commit equals one rebuild.
#[test]
fn replay_equals_from_scratch_at_every_commit() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    let route = |p: &str| {
        format!("import express from \"express\";\nconst app = express();\napp.get(\"{p}\", h);\n")
    };
    let call = |p: &str| format!("export const a = () => fetch(\"http://orders:8080{p}\");\n");

    let w_inc = workspace(&f, &d.path().join("inc"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    // (route file content, call file content) after each commit; None: absent.
    type Step = (Option<&'static str>, Option<&'static str>);
    let script: [Step; 12] = [
        (None, None),
        (Some("/a/:id"), None),
        (Some("/a/:id"), Some("/a/1")),
        (Some("/a/:id"), Some("/a/1")),
        (Some("/b/:id"), Some("/a/1")),
        (Some("/b/:id"), Some("/b/1")),
        (None, Some("/b/1")),
        (Some("/b/:id"), Some("/b/1")),
        (Some("/b/:id"), None),
        (Some("/b/:id"), Some("/b/2")),
        (Some("/c"), Some("/b/2")),
        (Some("/c"), Some("/c")),
    ];
    let mut model: Vec<BTreeSet<String>> = Vec::new();
    for (i, (r, c)) in script.iter().enumerate() {
        match r {
            Some(p) => f.write("orders/src/server.ts", &route(p)),
            None if i > 0 => drop(f.git(&["rm", "-q", "-f", "orders/src/server.ts"])),
            None => {}
        }
        match c {
            Some(p) => f.write("web/src/api.ts", &call(p)),
            None if i > 0 && script[i - 1].1.is_some() => {
                drop(f.git(&["rm", "-q", "-f", "web/src/api.ts"]))
            }
            None => {}
        }
        f.write("note.txt", &format!("{i}\n"));
        f.commit(&format!("step {i}"));
        // The truth: a link while a route and a call that match are both there.
        let hit = matches!(
            (r, c),
            (Some("/a/:id"), Some("/a/1"))
                | (Some("/b/:id"), Some("/b/1" | "/b/2"))
                | (Some("/c"), Some("/c"))
        );
        model.push(if hit {
            ["web/src/api.ts -> orders/src/server.ts".to_owned()].into()
        } else {
            BTreeSet::new()
        });
        run(&w_inc, &mut cache);
    }
    let w_once = workspace(&f, &d.path().join("once"));
    run(&w_once, &mut cache);
    assert_eq!(links(&w_inc), links(&w_once), "sync equals rebuild");

    // Live at each commit, read back from the intervals.
    let ix = Index::open_read(&w_once.index_path()).unwrap();
    let at = |seq: i64| -> BTreeSet<String> {
        ix.conn
            .prepare(
                "SELECT (SELECT path FROM file_path p WHERE p.file = ns.file ORDER BY from_seq DESC LIMIT 1)
                        || ' -> ' ||
                        (SELECT path FROM file_path p WHERE p.file = nd.file ORDER BY from_seq DESC LIMIT 1)
                 FROM interval i JOIN link l ON l.id = i.link
                   JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
                 WHERE l.kind = 'http' AND i.from_seq <= ?1 AND (i.to_seq IS NULL OR i.to_seq > ?1)",
            )
            .unwrap()
            .query_map([seq], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for (seq, want) in model.iter().enumerate() {
        assert_eq!(&at(seq as i64), want, "links live at commit {seq}");
    }
}
