//! SRV-05's done-when, against a real listener with raw requests: bind refusal, Host, Origin,
//! token, no CORS, and no reachability on the LAN address.

mod common;

use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

use cartograph_server::{ServeError, ServeOptions, bind, running};
use common::*;

/// The API routes served so far; each must refuse a request without the token.
const API: &[&str] = &[
    "/api/version",
    "/api/v1/meta",
    "/api/v1/workspaces",
    "/api/v1/workspaces/{ws}",
    "/api/v1/w/{ws}/freshness",
    "/api/v1/w/{ws}/repos",
    "/api/v1/w/{ws}/graph",
];

#[test]
fn a_non_loopback_host_is_refused_before_binding() {
    let d = tempfile::tempdir().unwrap();
    let rt = runtime();
    for host in ["0.0.0.0", "::", "192.168.1.5", "10.1.2.3", "example.com"] {
        let e = rt
            .block_on(bind(ServeOptions {
                data_dir: d.path().to_owned(),
                host: Some(host.into()),
                port: None,
                home: None,
            }))
            .err()
            .expect("refused");
        assert!(matches!(e, ServeError::NotLoopback { .. }), "{host}: {e}");
    }
    assert!(!d.path().join("server.json").exists(), "nothing written");
    let s = rt.block_on(bind(ServeOptions {
        data_dir: d.path().to_owned(),
        host: Some("::1".into()),
        port: None,
        home: None,
    }));
    // A machine without IPv6 can't bind ::1; when it can, it binds loopback only.
    if let Ok(s) = s {
        assert!(s.info().host.is_loopback());
        assert!(s.info().url().starts_with("http://[::1]:"));
    }
}

#[test]
fn the_listener_is_not_reachable_on_the_lan_address() {
    let r = start();
    // Learn this machine's outward address without sending anything (UDP connect is local).
    let lan = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip());
    match lan {
        Ok(ip) if !ip.is_loopback() && !ip.is_unspecified() => {
            let got = TcpStream::connect_timeout(
                &SocketAddr::new(ip, r.info.port),
                Duration::from_secs(2),
            );
            assert!(got.is_err(), "the server answered on {ip}");
        }
        other => eprintln!("no LAN address here ({other:?}); only the loopback bind is checked"),
    }
    assert!(r.info.host.is_loopback());
}

#[test]
fn a_foreign_host_gets_403_even_with_the_token() {
    let r = start();
    let p = r.info.port;
    let t = &r.info.token;
    // DNS rebinding: the right IP and port, someone else's name.
    for host in [
        format!("evil.example:{p}"),
        "evil.example".into(),
        format!("127.0.0.1:{}", p.wrapping_add(1)),
        format!("127.0.0.1.evil.example:{p}"),
        format!("192.168.1.5:{p}"),
        "127.0.0.1".into(),
    ] {
        let reply = raw(
            p,
            &format!("GET /api/v1/meta HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {t}"),
        );
        assert_eq!(
            (reply.status, reply.code().as_str()),
            (403, "forbidden_host"),
            "{host}"
        );
        let reply = raw(p, &format!("GET / HTTP/1.1\r\nHost: {host}"));
        assert_eq!(reply.status, 403, "static paths check Host too: {host}");
    }
    let no_host = raw(
        p,
        &format!("GET /api/v1/meta HTTP/1.0\r\nAuthorization: Bearer {t}"),
    );
    assert_eq!(no_host.status, 403);
    for host in [format!("127.0.0.1:{p}"), format!("localhost:{p}")] {
        let reply = raw(
            p,
            &format!("GET /api/v1/meta HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {t}"),
        );
        assert_eq!(reply.status, 200, "{host}");
    }
}

#[test]
fn a_foreign_origin_gets_403_on_every_method() {
    let r = start();
    let p = r.info.port;
    let t = &r.info.token;
    for origin in [
        "http://127.0.0.1:3000".to_owned(),
        format!("http://localhost:{p}"),
        format!("https://127.0.0.1:{p}"),
        "http://evil.example".into(),
        "null".into(),
    ] {
        for method in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            let reply = raw(
                p,
                &format!(
                    "{method} /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: {origin}\r\nAuthorization: Bearer {t}\r\nContent-Length: 0"
                ),
            );
            assert_eq!(
                (reply.status, reply.code().as_str()),
                (403, "forbidden_origin"),
                "{method} {origin}"
            );
            assert!(!reply.headers.contains_key("access-control-allow-origin"));
        }
    }
    let cross = raw(
        p,
        &format!(
            "GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nSec-Fetch-Site: cross-site\r\nAuthorization: Bearer {t}"
        ),
    );
    assert_eq!(
        (cross.status, cross.code().as_str()),
        (403, "forbidden_origin")
    );
    assert_eq!(get(&r, "/api/v1/meta").status, 200, "its own origin passes");
    // A link to the app from another site (chat, docs) is a cross-site navigation: the page loads,
    // since it holds no data; its API calls are same-origin and carry the token.
    let page = raw(
        p,
        &format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nSec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate"
        ),
    );
    assert_eq!(page.status, 200, "a cross-site navigation to a page loads");
}

/// "A cross-origin fetch from another local port can't read any response": its preflight gets
/// 405 and no CORS header, and a simple request is refused by Origin before any data is read.
#[test]
fn another_local_port_cannot_read_a_response() {
    let r = start();
    let p = r.info.port;
    let preflight = raw(
        p,
        &format!(
            "OPTIONS /api/v1/w/{}/graph HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:5173\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: authorization",
            r.ws
        ),
    );
    assert_eq!(preflight.status, 403);
    let own = raw(
        p,
        &format!("OPTIONS /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{p}"),
    );
    assert_eq!(
        (own.status, own.code().as_str()),
        (405, "method_not_allowed")
    );
    let simple = raw(
        p,
        &format!(
            "GET /api/v1/w/{}/graph HTTP/1.1\r\nHost: localhost:{p}\r\nOrigin: http://localhost:5173\r\nSec-Fetch-Site: same-site",
            r.ws
        ),
    );
    assert_eq!(simple.status, 403);
    for reply in [&preflight, &own, &simple] {
        assert!(
            reply
                .headers
                .keys()
                .all(|k| !k.starts_with("access-control-")),
            "{:?}",
            reply.headers
        );
        assert!(!reply.body.contains("\"data\""));
    }
}

#[test]
fn every_api_route_needs_the_token() {
    let r = start();
    let p = r.info.port;
    let host = format!("Host: 127.0.0.1:{p}");
    let wrong = format!("{}x", &r.info.token[..42]);
    for path in API {
        let path = path.replace("{ws}", &r.ws);
        for auth in [
            String::new(),
            format!("\r\nAuthorization: Bearer {wrong}"),
            format!("\r\nAuthorization: Bearer {}x", r.info.token),
            format!("\r\nAuthorization: bearer {}", r.info.token),
            format!("\r\nAuthorization: {}", r.info.token),
            format!("\r\nCookie: k={0}; token={0}", r.info.token),
            format!("\r\nX-Cartograph-Token: {}", r.info.token),
        ] {
            let reply = raw(p, &format!("GET {path} HTTP/1.1\r\n{host}{auth}"));
            assert_eq!(
                (reply.status, reply.code().as_str()),
                (401, "unauthorized"),
                "{path} {auth:?}"
            );
            assert!(!reply.body.contains("\"data\""));
        }
        let ok = raw(
            p,
            &format!(
                "GET {path} HTTP/1.1\r\n{host}\r\nAuthorization: Bearer {}",
                r.info.token
            ),
        );
        assert_eq!(ok.status, 200, "{path}: {}", ok.body);
        assert!(!ok.headers.contains_key("set-cookie"));
    }
    // Static paths need no token, only the Host check.
    assert_eq!(raw(p, &format!("GET / HTTP/1.1\r\n{host}")).status, 200);
}

#[test]
fn responses_carry_the_security_headers_and_no_cookies_or_cors() {
    let r = start();
    let reply = get(&r, &format!("/api/v1/w/{}/freshness", r.ws));
    assert_eq!(reply.status, 200);
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
        ("cache-control", "no-store"),
    ] {
        assert_eq!(reply.headers.get(k).map(String::as_str), Some(v), "{k}");
    }
    assert!(!reply.headers.contains_key("set-cookie"));
    assert!(!reply.headers.contains_key("access-control-allow-origin"));
    assert_eq!(reply.json()["data"]["state"], "current");
}

#[test]
fn server_json_is_private_reused_and_removed_and_the_token_changes() {
    let (dir, data, ws) = data_folder();
    let r = start_in(dir, data.clone(), ws);
    let path = data.join("server.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let found = running(&data).expect("the running server is found");
    assert_eq!(found, r.info);
    assert!(
        found
            .url()
            .starts_with(&format!("http://127.0.0.1:{}/#k=", r.info.port))
    );
    let (port, token) = (r.info.port, r.info.token.clone());
    // Keep the old server.json, as a crash would, to see the port reused on the next start.
    let kept = std::fs::read(&path).unwrap();
    let _dir = r.stop();
    assert!(!path.exists(), "removed on shutdown");
    assert!(running(&data).is_none());
    std::fs::write(&path, kept).unwrap();
    let rt = runtime();
    let s = rt
        .block_on(bind(ServeOptions {
            data_dir: data.clone(),
            host: None,
            port: None,
            home: None,
        }))
        .unwrap();
    assert_eq!(s.info().port, port, "the last port is reused when free");
    assert_ne!(s.info().token, token, "a new token each start");
}

/// A page at localhost or [::1] is redirected to the one origin with its path and query (308, so
/// the browser keeps the `#k=` fragment); the API is never redirected, and a foreign Host is still
/// refused before any redirect.
#[test]
fn page_routes_on_other_loopback_names_redirect_to_the_one_origin() {
    let r = start();
    let p = r.info.port;
    let t = &r.info.token;
    for host in [format!("localhost:{p}"), format!("[::1]:{p}")] {
        for path in ["/", "/map/?view=graph"] {
            let reply = raw(p, &format!("GET {path} HTTP/1.1\r\nHost: {host}"));
            assert_eq!(reply.status, 308, "{host}{path}");
            assert_eq!(
                reply.headers.get("location").map(String::as_str),
                Some(format!("http://127.0.0.1:{p}{path}").as_str()),
                "{host}{path}"
            );
        }
        let api = raw(
            p,
            &format!("GET /api/v1/meta HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {t}"),
        );
        assert_ne!(api.status, 308, "the API is not redirected: {host}");
        let foreign = raw(
            p,
            &format!(
                "GET /api/v1/meta HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nAuthorization: Bearer {t}"
            ),
        );
        assert_eq!(foreign.code(), "forbidden_origin", "{host}");
    }
    let own = raw(p, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}"));
    assert_eq!(own.status, 200);
    let evil = raw(p, "GET / HTTP/1.1\r\nHost: evil.example");
    assert_eq!(evil.status, 403, "Host is checked before any redirect");
}
