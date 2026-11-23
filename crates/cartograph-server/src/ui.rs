//! SRV-06: the app, embedded. Every page and asset comes from a map compiled into the binary
//! (`build.rs`), brotli-compressed and decompressed once into memory on first use. A request path is
//! only ever a key into that map, so no request can name a file on disk; `..`, backslashes and NUL
//! are refused before the lookup anyway.

use std::collections::HashMap;
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub struct Asset {
    pub bytes: Vec<u8>,
    pub etag: String,
}

/// Used when the export has no `csp.json` (the placeholder page has no inline script). Styles stay
/// `'unsafe-inline'`; scripts/csp.mjs says why hashes can't replace it.
const DEFAULT_POLICY: &str = "default-src 'self'; script-src 'self' {hashes}; style-src 'self' \
    'unsafe-inline'; connect-src 'self'; img-src 'self' data: blob:; font-src 'self'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

struct Ui {
    assets: HashMap<&'static str, Asset>,
    /// The CSP for each HTML key, built from `csp.json`'s policy and that page's script hashes.
    csp: HashMap<String, String>,
}

fn ui() -> &'static Ui {
    static UI: OnceLock<Ui> = OnceLock::new();
    UI.get_or_init(|| {
        let assets: HashMap<_, _> = ASSETS
            .iter()
            .map(|(key, br, hash)| {
                let mut bytes = Vec::new();
                brotli_decompressor::BrotliDecompress(&mut &br[..], &mut bytes)
                    .expect("the build script compressed this");
                let etag = format!("\"{hash:016x}\"");
                (*key, Asset { bytes, etag })
            })
            .collect();
        let manifest: serde_json::Value = assets
            .get("csp.json")
            .and_then(|a| serde_json::from_slice(&a.bytes).ok())
            .unwrap_or_default();
        let csp = assets
            .keys()
            .filter(|k| k.ends_with(".html"))
            .map(|k| ((*k).to_owned(), page_csp(&manifest, k)))
            .collect();
        Ui { assets, csp }
    })
}

/// One page's CSP: `csp.json`'s policy with `{hashes}` replaced by that page's inline-script hashes.
fn page_csp(manifest: &serde_json::Value, key: &str) -> String {
    let policy = manifest["policy"].as_str().unwrap_or(DEFAULT_POLICY);
    let hashes: Vec<&str> = manifest["hashes"][key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h.as_str())
        .collect();
    policy
        .replace("{hashes}", &hashes.join(" "))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ;", ";")
}

/// Every embedded key, for tests and the link check.
pub fn keys() -> impl Iterator<Item = &'static str> {
    ASSETS.iter().map(|(k, _, _)| *k)
}

/// The embedded key for a request path, or `None` for 404. `path` is still percent-encoded.
fn key_for(path: &str) -> Option<String> {
    let decoded = percent_decode(path)?;
    let rel = decoded.strip_prefix('/')?;
    if rel.contains('\\') || rel.contains('\0') || rel.split('/').any(|s| s == ".." || s == ".") {
        return None;
    }
    let assets = &ui().assets;
    let rel = rel.trim_end_matches('/');
    if rel.is_empty() {
        return Some("index.html".into());
    }
    // `/ask` is `ask.html`; `/_next/static/x.js` and `/favicon.ico` are themselves.
    [
        rel.to_owned(),
        format!("{rel}.html"),
        format!("{rel}/index.html"),
    ]
    .into_iter()
    .find(|k| assets.contains_key(k.as_str()))
}

/// `%XX` decoding. `None` when the result isn't UTF-8 or an escape is malformed.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn content_type(key: &str) -> &'static str {
    match key.rsplit_once('.').map(|(_, e)| e) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

/// Serves `path` from the embedded export: the page or asset with its headers, or `404.html`
/// with status 404.
pub fn serve(path: &str, req: &HeaderMap, head: bool) -> Response {
    let ui = ui();
    let (key, status) = match key_for(path) {
        Some(k) => (k, StatusCode::OK),
        None => ("404.html".to_owned(), StatusCode::NOT_FOUND),
    };
    let Some(asset) = ui.assets.get(key.as_str()) else {
        let mut res = Response::new(Body::from("not found\n"));
        *res.status_mut() = StatusCode::NOT_FOUND;
        return res;
    };
    let html = key.ends_with(".html");
    let not_modified = status == StatusCode::OK
        && req
            .get(header::IF_NONE_MATCH)
            .is_some_and(|v| v.as_bytes() == asset.etag.as_bytes());
    let body = if head || not_modified {
        Body::empty()
    } else {
        Body::from(&asset.bytes[..])
    };
    let mut res = Response::new(body);
    *res.status_mut() = if not_modified {
        StatusCode::NOT_MODIFIED
    } else {
        status
    };
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&key)),
    );
    let cache = if key.starts_with("_next/static/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&asset.etag) {
        h.insert(header::ETAG, v);
    }
    if html {
        if let Some(v) = ui.csp.get(&key).and_then(|p| HeaderValue::from_str(p).ok()) {
            h.insert(header::CONTENT_SECURITY_POLICY, v);
        }
        h.insert(
            header::HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        );
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_and_odd_paths_never_reach_a_key() {
        for bad in [
            "/..%2f..%2fetc/passwd",
            "/../etc/passwd",
            "/%2e%2e/%2e%2e/etc/passwd",
            "/_next/../index.html",
            "/a%5c..%5cindex.html",
            "/index.html%00",
            "/%00",
            "/./index.html",
            "/%zz",
            "/%c3",
            "relative",
        ] {
            assert_eq!(key_for(bad), None, "{bad}");
        }
        assert_eq!(key_for("/").as_deref(), Some("index.html"));
        assert_eq!(key_for("/index.html").as_deref(), Some("index.html"));
        assert_eq!(key_for("/nope"), None);
    }

    #[test]
    fn every_embedded_key_is_served_by_its_own_path() {
        for k in keys() {
            assert_eq!(key_for(&format!("/{k}")).as_deref(), Some(k), "{k}");
        }
    }

    #[test]
    fn each_page_gets_a_csp_with_its_own_hashes() {
        let m = serde_json::json!({
            "policy": "script-src 'self' {hashes}; connect-src 'self'",
            "hashes": {"ask.html": ["'sha256-a'", "'sha256-b'"], "index.html": []},
        });
        assert_eq!(
            page_csp(&m, "ask.html"),
            "script-src 'self' 'sha256-a' 'sha256-b'; connect-src 'self'"
        );
        assert_eq!(
            page_csp(&m, "index.html"),
            "script-src 'self'; connect-src 'self'"
        );
        // The embedded export: every page has a policy, none allows inline script or other hosts.
        let ui = ui();
        assert!(ui.csp.contains_key("index.html") && ui.csp.contains_key("404.html"));
        for (k, p) in &ui.csp {
            let script = p
                .split(';')
                .find(|d| d.trim().starts_with("script-src"))
                .unwrap();
            assert!(
                !script.contains("unsafe-inline") && !p.contains("{hashes}"),
                "{k}: {p}"
            );
            for d in [
                "connect-src 'self'",
                "frame-ancestors 'none'",
                "base-uri 'none'",
            ] {
                assert!(p.contains(d), "{k}: {d}");
            }
        }
    }

    #[test]
    fn content_types_cover_the_export() {
        assert_eq!(content_type("a/b.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("x.woff2"), "font/woff2");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
