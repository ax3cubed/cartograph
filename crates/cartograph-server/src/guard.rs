//! The checks every request passes, in SRV-05's order: Host allow list, Origin, `Sec-Fetch-Site`,
//! then OPTIONS (405, no CORS), the bearer token on `/api/*`, a write's Origin and JSON type, the concurrency limit and the
//! timeout. Cookies are never read.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use cartograph_api::types::{ApiError, ErrorCode};

use crate::routes::{App, error};

/// SRV-05: a 30 s timeout and 64 requests at once.
pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const CONCURRENCY: usize = 64;

fn refuse(code: ErrorCode, what: &str, why: &str, action: &str) -> Response {
    error(&ApiError::new(code, what, why, action))
}

pub async fn guard(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let mut res = check(&app, req, next).await;
    let h = res.headers_mut();
    for (k, v) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (
            header::HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    // The app's pages and assets set their own (SRV-06); everything else is never stored.
    h.entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    res
}

async fn check(app: &App, req: Request, next: Next) -> Response {
    let headers = req.headers();
    // DNS rebinding: a page on evil.example that resolves to 127.0.0.1 still sends its own Host.
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if !host.is_some_and(|h| app.hosts.iter().any(|a| a == h)) {
        return refuse(
            ErrorCode::ForbiddenHost,
            "this Host isn't served here",
            "the server answers only 127.0.0.1, localhost and [::1] on its own port",
            "open the address cartograph open printed",
        );
    }
    // localhost and [::1] name the same server, but a page loaded there would call the API from
    // another origin and be refused. A page is sent to the one origin with its path and query; 308
    // keeps the method and the browser keeps the `#k=` fragment, so the token survives. The API
    // gets no redirect: a client that names another host is refused by Origin as before.
    let path = req.uri().path();
    let api = path == "/api" || path.starts_with("/api/");
    if !api && host.is_some_and(|h| Some(h) != app.origin.strip_prefix("http://")) {
        let to = req.uri().path_and_query().map_or("/", |p| p.as_str());
        let mut res = Response::new(axum::body::Body::empty());
        *res.status_mut() = StatusCode::PERMANENT_REDIRECT;
        if let Ok(v) = HeaderValue::from_str(&format!("{}{to}", app.origin)) {
            res.headers_mut().insert(header::LOCATION, v);
        }
        return res;
    }
    if headers
        .get(header::ORIGIN)
        .is_some_and(|o| o.as_bytes() != app.origin.as_bytes())
    {
        return refuse(
            ErrorCode::ForbiddenOrigin,
            "requests from other origins are refused",
            "only the app served from this address may call it",
            "open the address cartograph open printed",
        );
    }
    // API only (A3): a page is data-free (the token is in the fragment, CSP applies), and links to
    // the app from a chat, a doc or an editor arrive as cross-site navigations.
    if req.uri().path().starts_with("/api/")
        && headers
            .get("sec-fetch-site")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"cross-site"))
    {
        return refuse(
            ErrorCode::ForbiddenOrigin,
            "cross-site requests are refused",
            "the browser marked this request as coming from another site",
            "open the address cartograph open printed",
        );
    }
    if req.method() == Method::OPTIONS {
        return refuse(
            ErrorCode::MethodNotAllowed,
            "OPTIONS isn't served",
            "there is no CORS: only the app's own origin may call the API",
            "call the API from the app",
        );
    }
    if req.uri().path().starts_with("/api/") && !authorized(headers, &app.token) {
        return refuse(
            ErrorCode::Unauthorized,
            "the session token is missing or wrong",
            "every API call needs Authorization: Bearer with this server's token",
            "open the address cartograph open printed; the token changes each start",
        );
    }
    // SRV-14: a write also needs its Origin (every browser sends one on POST, PUT and DELETE) and,
    // with a body, Content-Type: application/json, which no HTML form can send.
    if req.uri().path().starts_with("/api/") && !matches!(*req.method(), Method::GET | Method::HEAD)
    {
        if !headers.contains_key(header::ORIGIN) {
            return refuse(
                ErrorCode::ForbiddenOrigin,
                "a write without an Origin is refused",
                "only the app served from this address may change anything",
                "make the change from the app",
            );
        }
        let json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
        if *req.method() != Method::DELETE && !json {
            return refuse(
                ErrorCode::UnsupportedMediaType,
                "a write must be JSON",
                "writes need Content-Type: application/json",
                "send the body as JSON with that header",
            );
        }
    }
    let Ok(_permit) = app.slots.clone().try_acquire_owned() else {
        return refuse(
            ErrorCode::TooManyRequests,
            "too many requests at once",
            "the server answers 64 requests at a time",
            "retry in a moment",
        );
    };
    // The timeout covers producing the response head; a body streams after it. The event stream
    // is exempt anyway, so it is never cut by a slow first state read (SRV-13).
    if req.uri().path() == "/api/v1/events" {
        return next.run(req).await;
    }
    match tokio::time::timeout(TIMEOUT, next.run(req)).await {
        Ok(res) => res,
        Err(_) => refuse(
            ErrorCode::Timeout,
            "the request ran past 30 s",
            "each request has a time limit",
            "narrow the request and retry",
        ),
    }
}

/// `Authorization: Bearer <token>`, compared in constant time.
fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let Some(given) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.as_bytes().strip_prefix(b"Bearer "))
    else {
        return false;
    };
    let want = token.as_bytes();
    // The token's length is public (43); the comparison of its bytes is not data-dependent.
    given.len() == want.len()
        && given
            .iter()
            .zip(want)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}
