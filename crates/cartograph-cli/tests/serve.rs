//! HD-11: the local-server and privacy suite, black-box against the running binary
//! (`cartograph serve`) with raw HTTP/1.1. `cartograph-server`'s own tests check each guard in
//! process; this suite checks what a user's machine actually runs, and generates its route cases
//! from the route table, so a route added later is covered without editing this file.
//!
//! The egress check wraps this suite in CI: `scripts/egress.sh` runs it under strace and fails on
//! any connect() to an address that isn't loopback.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use cartograph_api::routes::{Kind, routes};

/// A key that must never leave the process environment: not into the data folder, the logs or a
/// response.
const CANARY: &str = "sk-cartograph-canary-7f3a9c1e5b";

struct Served {
    child: Child,
    home: PathBuf,
    port: u16,
    token: String,
    ws: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cartograph"));
    c.env_remove("CLICOLOR_FORCE")
        // The canary rides in every variable an AI provider key would.
        .env("OPENAI_API_KEY", CANARY)
        .env("ANTHROPIC_API_KEY", CANARY)
        .env("CARTOGRAPH_CANARY", CANARY)
        .stdin(Stdio::null());
    c
}

/// A data folder holding the northwind golden index as workspace `northwind`.
fn home() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let ws = d.path().join("home/workspaces/northwind");
    std::fs::create_dir_all(&ws).unwrap();
    let built = tempfile::tempdir().unwrap();
    let ix = cartograph_fixtures::index(&cartograph_fixtures::northwind(), built.path());
    std::fs::copy(ix, ws.join("index.cartograph")).unwrap();
    d
}

/// Starts `cartograph serve` on `home` and waits for `server.json`.
fn serve(home: &Path, args: &[&str]) -> Served {
    serve_from(home, home, args)
}

/// Starts `cartograph serve` with `CARTOGRAPH_HOME=env_home` and waits for `server.json` in `home`.
/// `HOME` points at `env_home` too, so the folder picker and the registry never see the real one.
fn serve_from(env_home: &Path, home: &Path, args: &[&str]) -> Served {
    let child = bin()
        .arg("serve")
        .args(args)
        .env("CARTOGRAPH_HOME", env_home)
        .env("HOME", env_home)
        .env("USERPROFILE", env_home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Owned by `Served` from here, whose Drop kills and waits, so no path leaves a zombie.
    let mut s = Served {
        child,
        home: home.to_owned(),
        port: 0,
        token: String::new(),
        ws: "northwind".into(),
    };
    let file = home.join("server.json");
    let t = Instant::now();
    loop {
        if let Ok(b) = std::fs::read(&file)
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b)
        {
            let port = v["port"].as_u64().unwrap() as u16;
            let host: std::net::IpAddr = v["host"].as_str().unwrap().parse().unwrap();
            if TcpStream::connect((host, port)).is_ok() {
                s.port = port;
                s.token = v["token"].as_str().unwrap().to_owned();
                return s;
            }
        }
        assert!(t.elapsed() < Duration::from_secs(20), "the server started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

/// Sends `head` (request line and headers, no blank line) and `body`, and reads the whole reply.
fn send(port: u16, head: &str, body: &[u8]) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let _ = s.write_all(format!("{head}\r\nConnection: close\r\n\r\n").as_bytes());
    let _ = s.write_all(body);
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (top, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = top.lines();
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    Reply {
        status,
        headers: lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect(),
        body: body.to_owned(),
    }
}

/// Every route in the table with its path parameters filled in.
fn table(ws: &str) -> Vec<(&'static str, String, Kind)> {
    routes()
        .into_iter()
        .map(|r| {
            let path = r
                .path
                .split('/')
                .map(|seg| match seg {
                    "{ws}" => ws.to_owned(),
                    s if s.starts_with('{') => "x".to_owned(),
                    s => s.to_owned(),
                })
                .collect::<Vec<_>>()
                .join("/");
            (r.method, path, r.kind)
        })
        .collect()
}

fn no_data(r: &Reply) -> bool {
    !r.body.contains("\"data\"") && !r.body.contains("northwind/") && !r.body.contains(CANARY)
}

#[test]
fn a_non_loopback_host_never_binds() {
    let d = tempfile::tempdir().unwrap();
    for host in ["0.0.0.0", "::", "192.168.1.5", "example.com"] {
        let o = bin()
            .args(["serve", "--host", host])
            .env("CARTOGRAPH_HOME", d.path())
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(1), "{host}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("loopback"),
            "{host}"
        );
        assert!(!d.path().join("server.json").exists(), "{host}");
    }
    // ::1 behaves like 127.0.0.1 where the machine has IPv6.
    if std::net::TcpListener::bind("[::1]:0").is_ok() {
        let h = home();
        let s = serve(&h.path().join("home"), &["--host", "::1"]);
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(s.home.join("server.json")).unwrap()).unwrap();
        assert_eq!(v["host"], "::1");
    }
}

/// DNS rebinding, a foreign Origin, cross-site fetch metadata and a missing or wrong token, on every
/// route and method in the table (SSE and Ask streams included) and on paths outside it. None may
/// answer 2xx or carry data.
#[test]
fn every_route_refuses_foreign_hosts_origins_and_missing_tokens() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let (p, t) = (s.port, &s.token);
    let mut cases = table(&s.ws);
    cases.push(("GET", "/api/v1/not-a-route".into(), Kind::Read));
    cases.push(("PATCH", "/api/v1/meta".into(), Kind::Write));
    for (method, path, _) in &cases {
        let own = format!("Host: 127.0.0.1:{p}");
        for (head, want) in [
            (
                format!("Host: evil.example:{p}\r\nAuthorization: Bearer {t}"),
                403,
            ),
            (
                format!("Host: localhost.evil.example:{p}\r\nAuthorization: Bearer {t}"),
                403,
            ),
            (
                format!("{own}\r\nOrigin: http://evil.example\r\nAuthorization: Bearer {t}"),
                403,
            ),
            (
                format!("{own}\r\nOrigin: http://localhost:{p}\r\nAuthorization: Bearer {t}"),
                403,
            ),
            (
                format!("{own}\r\nSec-Fetch-Site: cross-site\r\nAuthorization: Bearer {t}"),
                403,
            ),
            (own.clone(), 401),
            (format!("{own}\r\nCookie: token={t}; k={t}"), 401),
            (
                format!("{own}\r\nAuthorization: Bearer {}", "A".repeat(43)),
                401,
            ),
        ] {
            let r = send(
                p,
                &format!("{method} {path} HTTP/1.1\r\n{head}\r\nContent-Length: 0"),
                b"",
            );
            assert_eq!(r.status, want, "{method} {path} with {head:?}: {}", r.body);
            assert!(no_data(&r), "{method} {path}: {}", r.body);
            assert!(
                r.headers.keys().all(|k| !k.starts_with("access-control-")),
                "no CORS: {method} {path}"
            );
            assert!(!r.headers.contains_key("set-cookie"));
        }
    }
}

#[test]
fn the_token_changes_on_every_start() {
    let h = home();
    let home = h.path().join("home");
    let a = serve(&home, &[]);
    let (port, token) = (a.port, a.token.clone());
    drop(a);
    let _ = std::fs::remove_file(home.join("server.json"));
    let b = serve(&home, &["--port", &port.to_string()]);
    assert_ne!(b.token, token);
    // The old token is refused by the new server.
    let r = send(
        b.port,
        &format!(
            "GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}",
            b.port
        ),
        b"",
    );
    assert_eq!(r.status, 401);
}

/// CSRF: a cross-site HTML form post and a no-preflight `fetch` (text/plain, `Origin: null` from a
/// sandboxed frame) both fail on every write route, as do the same requests carrying no token.
#[test]
fn cross_site_form_posts_and_simple_fetches_fail_on_every_write_route() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let p = s.port;
    let writes: Vec<_> = table(&s.ws)
        .into_iter()
        .filter(|(m, ..)| *m != "GET")
        .collect();
    assert!(!writes.is_empty());
    for (method, path, _) in &writes {
        for extra in [
            "Origin: http://evil.example\r\nSec-Fetch-Site: cross-site\r\nContent-Type: application/x-www-form-urlencoded",
            "Origin: null\r\nContent-Type: text/plain",
            "Sec-Fetch-Site: same-site\r\nContent-Type: text/plain",
        ] {
            let body = b"a=1";
            let r = send(
                p,
                &format!(
                    "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n{extra}\r\nContent-Length: {}",
                    body.len()
                ),
                body,
            );
            assert!(
                matches!(r.status, 401 | 403),
                "{method} {path} {extra:?}: {} {}",
                r.status,
                r.body
            );
            assert!(no_data(&r));
        }
    }
}

/// The fixed list of write routes: a state-changing method is only ever accepted on a route the
/// table marks as a write, and a write route never accepts a body field it doesn't know.
#[test]
fn only_the_listed_write_routes_accept_a_state_changing_method() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let (p, t) = (s.port, s.token.clone());
    let all = table(&s.ws);
    let listed: Vec<(&str, &str)> = all
        .iter()
        .filter(|(m, ..)| *m != "GET")
        .map(|(m, p, _)| (*m, p.as_str()))
        .collect();
    let req = |method: &str, path: &str, body: &str| {
        send(
            p,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nAuthorization: Bearer {t}\r\nContent-Type: application/json\r\nContent-Length: {}",
                body.len()
            ),
            body.as_bytes(),
        )
    };
    // Every route path, every state-changing method that isn't listed for it: never 2xx.
    for (_, path, _) in &all {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            if listed.contains(&(method, path.as_str())) {
                continue;
            }
            let r = req(method, path, "{}");
            assert!(
                matches!(r.status, 404 | 405),
                "{method} {path} isn't in the write list but answered {}: {}",
                r.status,
                r.body
            );
        }
    }
    // Every listed write: an unknown field is refused (closed schemas, HD-04), never applied. A
    // route the binary routes answers anything but the router's own 404 or 405.
    let mut served = Vec::new();
    for (method, path) in &listed {
        let r = req(method, path, r#"{"__not_a_field__":true}"#);
        assert!(
            !(200..300).contains(&r.status),
            "{method} {path} accepted an unknown field: {} {}",
            r.status,
            r.body
        );
        let unrouted = r.status == 405 || (r.status == 404 && r.body.contains("no such route"));
        if !unrouted {
            served.push(format!("{method} {path}"));
        }
    }
    // The fixed list of write routes the binary serves today (A3). A new write route changes this
    // list, the Interfaces section and HD-11 together.
    let ws = &s.ws;
    let mut want = vec![
        "PUT /api/v1/prefs".to_owned(),
        "POST /api/v1/discover".into(),
        "POST /api/v1/workspaces".into(),
        format!("POST /api/v1/w/{ws}/jobs"),
        "DELETE /api/v1/jobs/x".into(),
        format!("POST /api/v1/w/{ws}/config/preview"),
        format!("PUT /api/v1/w/{ws}/config"),
    ];
    want.sort();
    served.sort();
    assert_eq!(served, want, "the served write routes changed");
}

/// Static paths never reach the disk: traversal in every spelling, absolute paths, NUL, and the
/// data folder's own files (a symlink planted there included) all get 404.
#[test]
fn static_paths_never_reach_the_disk() {
    let h = home();
    let home = h.path().join("home");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", home.join("passwd")).unwrap();
    let s = serve(&home, &[]);
    let p = s.port;
    for path in [
        "/../../../../etc/passwd",
        "/..%2f..%2f..%2f..%2fetc/passwd",
        "/%2e%2e/%2e%2e/%2e%2e/etc/passwd",
        "/..%5c..%5c..%5cwindows/win.ini",
        "//etc/passwd",
        "/C:/Windows/win.ini",
        "/%00",
        "/index.html%00.txt",
        "/passwd",
        "/server.json",
        "/workspaces/northwind/index.cartograph",
    ] {
        let r = send(
            p,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}"),
            b"",
        );
        assert_eq!(r.status, 404, "{path}");
        assert!(
            !r.body.contains("root:") && !r.body.contains(&s.token) && !r.body.contains("SQLite"),
            "{path}"
        );
    }
}

/// The security headers on an exported page and on the API, from the binary as shipped.
#[test]
fn pages_and_api_carry_the_security_headers() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let p = s.port;
    let page = send(p, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}"), b"");
    assert_eq!(page.status, 200);
    let csp = page
        .headers
        .get("content-security-policy")
        .expect("a CSP on every page");
    for want in [
        "default-src 'self'",
        "frame-ancestors 'none'",
        "connect-src 'self'",
    ] {
        assert!(csp.contains(want), "{want} in {csp}");
    }
    assert!(!csp.contains("unsafe-eval"), "{csp}");
    let api = send(
        p,
        &format!(
            "GET /api/v1/w/{}/graph HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nAuthorization: Bearer {}",
            s.ws, s.token
        ),
        b"",
    );
    assert_eq!(api.status, 200, "{}", api.body);
    for r in [&page, &api] {
        for (k, v) in [
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("cross-origin-resource-policy", "same-origin"),
        ] {
            assert_eq!(r.headers.get(k).map(String::as_str), Some(v), "{k}");
        }
    }
}

/// Resource limits: a body over 1 MiB is refused before it is read in full, and a client that
/// never finishes its headers doesn't keep a connection slot.
#[test]
fn an_oversized_body_is_refused() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let p = s.port;
    let big = vec![b'x'; (1 << 20) + 1024];
    // A served write route answers 413; until one is served, unrouted writes answer 404 or 405
    // without reading the body. Either way, never 2xx, and the server keeps answering.
    for (method, path, _) in table(&s.ws).into_iter().filter(|(m, ..)| *m != "GET") {
        let r = send(
            p,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}",
                s.token,
                big.len()
            ),
            &big,
        );
        assert!(
            matches!(r.status, 0 | 400 | 403 | 404 | 405 | 413),
            "{method} {path}: {}",
            r.status
        );
    }
    let ok = send(
        p,
        &format!(
            "GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nAuthorization: Bearer {}",
            s.token
        ),
        b"",
    );
    assert_eq!(ok.status, 200);
}

#[test]
fn a_client_that_never_finishes_its_headers_is_dropped() {
    let h = home();
    let s = serve(&h.path().join("home"), &[]);
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.write_all(b"GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    c.set_read_timeout(Some(Duration::from_secs(45))).unwrap();
    let t = Instant::now();
    let mut buf = [0u8; 64];
    let n = c.read(&mut buf).unwrap_or(0);
    assert!(
        n == 0 || buf.starts_with(b"HTTP/1.1 408"),
        "the server answered a half request"
    );
    assert!(t.elapsed() < Duration::from_secs(40), "dropped in time");
}

/// After a full run of requests, the planted canary key is nowhere in the data folder, the logs or
/// any response.
#[test]
fn the_canary_key_never_reaches_the_data_folder_logs_or_responses() {
    let h = home();
    let home = h.path().join("home");
    let s = serve(&home, &["-vvv"]);
    let p = s.port;
    let mut bodies = String::new();
    for (method, path, _) in table(&s.ws) {
        let r = send(
            p,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nAuthorization: Bearer {}\r\nContent-Length: 0",
                s.token
            ),
            b"",
        );
        bodies.push_str(&r.body);
        bodies.extend(r.headers.values().map(String::as_str));
    }
    drop(s);
    assert!(!bodies.contains(CANARY), "a response carries the canary");
    let mut stack = vec![home];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let t = e.file_type().unwrap();
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                let b = std::fs::read(e.path()).unwrap();
                assert!(
                    !b.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()),
                    "{} carries the canary",
                    e.path().display()
                );
            }
        }
    }
}

/// HD-04: `serve --fixture <dir>` (debug builds) serves a fixture data folder, not the user's.
#[cfg(debug_assertions)]
#[test]
fn serve_fixture_serves_that_data_folder() {
    let h = home();
    let fixture = h.path().join("home");
    let user = tempfile::tempdir().unwrap();
    let s = serve_from(
        user.path(),
        &fixture,
        &["--fixture", fixture.to_str().unwrap()],
    );
    let r = send(
        s.port,
        &format!(
            "GET /api/v1/workspaces HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}",
            s.port, s.token
        ),
        b"",
    );
    assert_eq!(r.status, 200);
    assert!(r.body.contains("\"northwind\""), "{}", r.body);
    assert!(!user.path().join("server.json").exists());
}
