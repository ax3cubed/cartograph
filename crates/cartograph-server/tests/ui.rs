//! SRV-06's done-when against a real listener: every route of the embedded export is served from
//! memory with its headers, unknown paths get 404.html, and traversal gets 404.

mod common;

use cartograph_server::ui;
use common::*;

fn page(r: &Running, path: &str) -> Reply {
    let p = r.info.port;
    raw(p, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}"))
}

#[test]
fn every_embedded_file_is_served_with_its_headers() {
    let r = start();
    // The data folder goes away: nothing below may read the disk for a page.
    let mut n = 0;
    for key in ui::keys() {
        let reply = page(&r, &format!("/{key}"));
        assert_eq!(reply.status, 200, "{key}");
        n += 1;
        let cache = &reply.headers["cache-control"];
        if key.starts_with("_next/static/") {
            assert_eq!(cache, "public, max-age=31536000, immutable", "{key}");
        } else {
            assert_eq!(cache, "no-cache", "{key}");
        }
        assert!(reply.headers["etag"].starts_with('"'), "{key}");
        assert_eq!(reply.headers["x-content-type-options"], "nosniff");
        if key.ends_with(".html") {
            assert!(reply.headers["content-type"].starts_with("text/html"));
            let csp = &reply.headers["content-security-policy"];
            assert!(csp.contains("connect-src 'self'"), "{key}: {csp}");
            assert!(csp.contains("frame-ancestors 'none'"), "{key}: {csp}");
            assert_eq!(reply.headers["cross-origin-opener-policy"], "same-origin");
        }
    }
    assert!(n >= 2, "index.html and 404.html at least");
    // Extensionless routes map to their page.
    let home = page(&r, "/");
    assert_eq!(home.status, 200);
    assert!(home.headers.contains_key("content-security-policy"));
    for key in ui::keys().filter(|k| k.ends_with(".html") && *k != "404.html") {
        let route = format!("/{}", key.trim_end_matches(".html"));
        assert_eq!(page(&r, &route).status, 200, "{route}");
    }
    // A revalidation with the ETag is answered 304.
    let etag = home.headers["etag"].clone();
    let p = r.info.port;
    let again = raw(
        p,
        &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nIf-None-Match: {etag}"),
    );
    assert_eq!(again.status, 304);
}

#[test]
fn unknown_paths_and_traversal_get_the_404_page() {
    let r = start();
    let p = r.info.port;
    for path in [
        "/..%2f..%2fetc/passwd",
        "/../../etc/passwd",
        "/%2e%2e/%2e%2e/etc/passwd",
        "/_next/static/..%2f..%2f..%2fCargo.toml",
        "/a%5c..%5c..%5cCargo.toml",
        "/index.html%00.js",
        "/no/such/page",
        "/Cargo.toml",
        "/server.json",
    ] {
        let reply = page(&r, path);
        assert_eq!(reply.status, 404, "{path}");
        assert!(
            reply.headers["content-type"].starts_with("text/html"),
            "{path}"
        );
        assert!(
            !reply.body.contains("[workspace]") && !reply.body.contains("token"),
            "{path}"
        );
    }
    // /api/… stays the JSON contract, and a page doesn't take a write.
    assert_eq!(get(&r, "/api/v1/nope").code(), "not_found");
    let post = raw(
        p,
        &format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nContent-Length: 0"),
    );
    assert_eq!(
        (post.status, post.code().as_str()),
        (405, "method_not_allowed")
    );
}
