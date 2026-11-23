//! SRV-13's done-when: a commit made while the app is open becomes an `index.changed` event with
//! no user action, and the map read after it has the new data; a cancel leaves the index readable;
//! two sync requests at once make one run.

mod common;

use std::time::{Duration, Instant};

use cartograph_git::fixture::Fixture;
use common::*;

fn post(r: &Running, path: &str, body: &str) -> Reply {
    write(r, "POST", path, Some(body))
}

#[test]
fn a_commit_while_the_app_is_open_becomes_an_event_and_new_data() {
    let r = start();
    let before = get(&r, &format!("/api/v1/w/{}/graph", r.ws)).json();
    let files = before["data"]["nodes"]["name"].as_array().unwrap().len();
    let mut ev = Events::open(&r);
    let first = ev.wait("freshness", 10);
    assert_eq!(first["ws"], r.ws.as_str());
    assert_eq!(first["freshness"]["state"], "current");

    // Re-running `git init` on the fixture repo only reattaches to it.
    let mut f = Fixture::init(&r.data.parent().unwrap().join("repo"));
    f.set_clock(1_700_000_000);
    f.write("src/d.ts", "import { b } from './b';\n");
    f.commit("three");

    // No request from here on: the ticker sees the repo behind and syncs in the background.
    let started = ev.wait("job.started", 30);
    assert_eq!(started["by"], "background");
    assert_eq!(started["kind"], "sync");
    let finished = ev.wait("job.finished", 60);
    assert_eq!(finished["ok"], true, "{finished}");
    let changed = ev.wait("index.changed", 30);
    assert_eq!(changed["ws"], r.ws.as_str());
    let after = get(&r, &format!("/api/v1/w/{}/graph", r.ws));
    assert_eq!(
        after.headers["cartograph-generation"],
        changed["generation"].to_string()
    );
    let now = after.json()["data"]["nodes"]["name"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(now, files + 1, "src/d.ts is on the map");
}

#[test]
fn two_requests_make_one_run_and_a_cancel_keeps_the_index_readable() {
    let r = start();
    let path = format!("/api/v1/w/{}/jobs", r.ws);
    let a = post(&r, &path, r#"{"kind":"rebuild","repo":null}"#);
    let b = post(&r, &path, r#"{"kind":"rebuild","repo":null}"#);
    assert_eq!(a.status, 200, "{}", a.body);
    let id = a.json()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        b.json()["data"]["id"],
        id.as_str(),
        "the running job, not a second"
    );

    let cancel = write(&r, "DELETE", &format!("/api/v1/jobs/{id}"), None);
    assert_eq!(cancel.status, 200, "{}", cancel.body);
    let t = Instant::now();
    let state = loop {
        let j = get(&r, &format!("/api/v1/jobs/{id}")).json();
        let s = j["data"]["state"].as_str().unwrap().to_owned();
        if s != "running" || t.elapsed() > Duration::from_secs(60) {
            break s;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(state, "cancelled");
    let read = get(&r, &format!("/api/v1/w/{}/graph", r.ws));
    assert_eq!(read.status, 200, "the previous index is still readable");

    // Only a sync or a rebuild can be started.
    let bad = post(&r, &path, r#"{"kind":"shell","repo":null}"#);
    assert_eq!((bad.status, bad.code().as_str()), (400, "bad_request"));
    let extra = post(&r, &path, r#"{"kind":"sync","repo":null,"cmd":"rm"}"#);
    assert_eq!((extra.status, extra.code().as_str()), (400, "bad_request"));
}

#[test]
fn at_most_16_streams() {
    let r = start();
    let open: Vec<Events> = (0..16).map(|_| Events::open(&r)).collect();
    let p = r.info.port;
    let seventeenth = raw(
        p,
        &format!(
            "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nAuthorization: Bearer {}",
            r.info.token
        ),
    );
    assert_eq!(
        (seventeenth.status, seventeenth.code().as_str()),
        (429, "too_many_requests")
    );
    drop(open);
}

#[test]
fn a_failed_index_says_why_and_what_to_run() {
    let r = start();
    // A sync killed part way: the index says syncing, but nobody holds the writer lock.
    let index = r
        .data
        .join("workspaces")
        .join(&r.ws)
        .join("index.cartograph");
    let c = cartograph_store::rusqlite::Connection::open(&index).unwrap();
    c.execute("UPDATE meta SET value = 'syncing' WHERE key = 'state'", [])
        .unwrap();
    drop(c);
    // Past the freshness check's 1 s memo.
    std::thread::sleep(Duration::from_millis(1100));
    let f = get(&r, &format!("/api/v1/w/{}/freshness", r.ws)).json();
    assert_eq!(f["data"]["state"], "failed", "{f}");
    let action = f["data"]["error"]["try"].as_str().unwrap_or_default();
    assert!(action.contains("cartograph sync"), "{f}");
    assert!(f["data"]["error"]["why"].as_str().is_some(), "{f}");
}

#[test]
fn the_ticker_s_first_look_sends_what_it_sees() {
    let r = start();
    let mut ev = Events::open(&r);
    let connect = ev.wait("freshness", 10);
    assert_eq!(connect["ws"], r.ws.as_str());
    // A sync that ends between the connect and the first tick changes the state the tab was sent;
    // the first tick sends what it sees rather than only recording it.
    let tick = ev.wait("freshness", 5);
    assert_eq!(tick["ws"], r.ws.as_str());
}

#[test]
fn rewritten_history_syncs_in_the_background() {
    let r = start();
    let mut ev = Events::open(&r);
    assert_eq!(ev.wait("freshness", 10)["freshness"]["state"], "current");
    // The indexed tip ("two") is no longer on the branch: history was rewritten, in one ref update.
    let mut f = Fixture::init(&r.data.parent().unwrap().join("repo"));
    f.set_clock(1_700_000_000);
    let one = f.git(&["rev-parse", "HEAD~1"]);
    f.write("src/e.ts", "export const e = 1;\n");
    f.commit_with_parents("two, again", &[&one]);
    let started = ev.wait("job.started", 30);
    assert_eq!(started["by"], "background");
    let finished = ev.wait("job.finished", 60);
    assert_eq!(finished["ok"], true, "{finished}");
    // The freshness check is memoised for 1 s, so the ticker reports current within a tick or two.
    let t = Instant::now();
    while ev.wait("freshness", 30)["freshness"]["state"] != "current" {
        assert!(t.elapsed() < Duration::from_secs(30), "never current again");
    }
}
