//! The panel, search, timeline and traversal reads (SRV-08 to SRV-11): thin wrappers over
//! `cartograph-query`, each answered in the shared envelope by [`read`] or [`read_page`].
//!
//! `/source` is the one read that leaves the index: it resolves a node and a commit to a blob id
//! through the index and reads that blob with gix, so it can't be pointed at a path or an object
//! of its own choosing (SRV-08 security).

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use cartograph_api::types::{ApiError, Bin, ErrorCode, ImpactRequest, ReadParams};
use cartograph_query::{At, ImpactInput, Page, QueryError, SearchQuery};
use serde::Deserialize;

use crate::routes::{Answer, App, body, error, read, read_page};

pub(crate) fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/api/v1/w/{ws}/nodes/{id}", get(node))
        .route("/api/v1/w/{ws}/nodes/{id}/children", get(children))
        .route("/api/v1/w/{ws}/nodes/{id}/metrics", get(metrics))
        .route("/api/v1/w/{ws}/nodes/{id}/series", get(series))
        .route("/api/v1/w/{ws}/nodes/{id}/neighbors", get(neighbors))
        .route("/api/v1/w/{ws}/nodes/{id}/changes", get(changes))
        .route("/api/v1/w/{ws}/nodes/{id}/ranking", get(ranking))
        .route("/api/v1/w/{ws}/nodes/{id}/history", get(history))
        .route("/api/v1/w/{ws}/nodes/{id}/symbols", get(symbols))
        .route(
            "/api/v1/w/{ws}/nodes/{id}/reading-order",
            get(reading_order),
        )
        .route("/api/v1/w/{ws}/links/{id}", get(link))
        .route("/api/v1/w/{ws}/source", get(source))
        .route("/api/v1/w/{ws}/search", get(search))
        .route("/api/v1/w/{ws}/commits", get(commits))
        .route("/api/v1/w/{ws}/commits/{sha}", get(commit))
        .route("/api/v1/w/{ws}/commits/{sha}/impact", get(commit_impact))
        .route("/api/v1/w/{ws}/timeline/density", get(density))
        .route("/api/v1/w/{ws}/timeline/markers", get(markers))
        .route("/api/v1/w/{ws}/impact", get(impact_get).post(impact_post))
        .route("/api/v1/w/{ws}/path", get(path))
        .route("/api/v1/w/{ws}/reach", get(reach))
        .route("/api/v1/w/{ws}/stories", get(stories))
        .route("/api/v1/w/{ws}/coverage", get(coverage))
        .route("/api/v1/w/{ws}/coverage/{row}/unresolved", get(unresolved))
        .route("/api/v1/w/{ws}/runs", get(runs))
        .route("/api/v1/w/{ws}/extractors", get(extractors))
        .route("/api/v1/w/{ws}/services", get(services))
        .route("/api/v1/w/{ws}/environments", get(environments))
        .route("/api/v1/w/{ws}/owners", get(owners))
        .route("/api/v1/w/{ws}/contracts", get(contracts))
        .route(
            "/api/v1/w/{ws}/contracts/{id}/versions",
            get(contract_versions),
        )
        .route("/api/v1/w/{ws}/contracts/{id}/diff", get(contract_diff))
        .route("/api/v1/mcp/tools", get(mcp_tools))
        .route("/api/v1/agents", get(agents))
        .route(
            "/api/v1/agents/preview",
            axum::routing::post(agents_preview),
        )
        .route(
            "/api/v1/agents/install",
            axum::routing::post(agents_install),
        )
}

/// A malformed query string is a 400 in the shared error shape.
fn params<T>(q: Result<Query<T>, QueryRejection>) -> Result<T, Response> {
    q.map(|Query(p)| p).map_err(|r| {
        error(&ApiError::new(
            ErrorCode::BadRequest,
            "the query string is malformed",
            &r.body_text(),
            "check the parameter names and values against the API reference",
        ))
    })
}

fn bad(e: QueryError) -> Response {
    error(&e.to_api())
}

fn at(p: Option<&str>) -> Result<At, Response> {
    At::parse(p.unwrap_or("now")).map_err(bad)
}

macro_rules! node_read {
    ($name:ident, |$s:ident, $id:ident, $at:ident, $p:ident| $body:expr) => {
        async fn $name(
            State(app): State<Arc<App>>,
            Path((ws, $id)): Path<(String, String)>,
            q: Result<Query<ReadParams>, QueryRejection>,
        ) -> Response {
            let $p = match params(q) {
                Ok(p) => p,
                Err(e) => return e,
            };
            let $at = match at($p.at.as_deref()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            read(app, ws, move |$s| $body).await
        }
    };
}

macro_rules! node_page {
    ($name:ident, |$s:ident, $id:ident, $at:ident, $page:ident| $body:expr) => {
        async fn $name(
            State(app): State<Arc<App>>,
            Path((ws, $id)): Path<(String, String)>,
            q: Result<Query<ReadParams>, QueryRejection>,
        ) -> Response {
            let p = match params(q) {
                Ok(p) => p,
                Err(e) => return e,
            };
            let ($at, $page) = match at(p.at.as_deref())
                .and_then(|a| Ok((a, Page::parse(p.limit, p.cursor.as_deref()).map_err(bad)?)))
            {
                Ok(x) => x,
                Err(e) => return e,
            };
            read_page(app, ws, move |$s| $body.map(Answer::from)).await
        }
    };
}

node_read!(node, |s, id, at, _p| s.node_detail(&id, &at));
node_page!(unresolved, |s, row, at, page| s.unresolved(&row, &at, page));
node_read!(metrics, |s, id, at, _p| s.metrics(&id, &at));
// `limit` is the number of monthly bins here: 12 when absent.
node_read!(series, |s, id, at, p| s.series(
    &id,
    &at,
    p.limit.unwrap_or(12) as usize
));
node_read!(history, |s, id, _at, _p| s.history(&id));
node_read!(symbols, |s, id, _at, _p| s.symbols(&id));
node_read!(link, |s, id, at, p| s.link_detail_in(
    &id,
    &at,
    p.env.as_deref()
));
node_page!(children, |s, id, at, page| s.children(&id, &at, page));
node_page!(neighbors, |s, id, at, page| s.neighbors(&id, &at, page));
node_page!(changes, |s, id, at, page| s.changes(&id, &at, page));
node_page!(ranking, |s, id, at, page| s.ranking(&id, &at, page));
node_page!(reading_order, |s, id, at, page| s
    .reading_order(&id, &at, page));

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    kinds: Option<String>,
    at: Option<String>,
    scope: Option<String>,
    limit: Option<u32>,
}

async fn search(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<SearchParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let query = match SearchQuery::from_params(
        p.q.as_deref(),
        p.kinds.as_deref(),
        p.at.as_deref(),
        p.scope.as_deref(),
        p.limit,
    ) {
        Ok(q) => q,
        Err(e) => return bad(e),
    };
    read(app, ws, move |s| s.search(&query)).await
}

/// Commits are paged at 100 by default (SRV-10).
async fn commits(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<ReadParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (at, page) = match at(p.at.as_deref()).and_then(|a| {
        Ok((
            a,
            Page::parse(Some(p.limit.unwrap_or(100)), p.cursor.as_deref()).map_err(bad)?,
        ))
    }) {
        Ok(x) => x,
        Err(e) => return e,
    };
    read_page(app, ws, move |s| s.commits(&at, page).map(Answer::from)).await
}

async fn commit(State(app): State<Arc<App>>, Path((ws, sha)): Path<(String, String)>) -> Response {
    read(app, ws, move |s| s.commit_by_sha(&sha)).await
}

async fn commit_impact(
    State(app): State<Arc<App>>,
    Path((ws, sha)): Path<(String, String)>,
) -> Response {
    read(app, ws, move |s| s.commit_impact(&sha)).await
}

#[derive(Deserialize)]
struct RangeParams {
    bin: Option<Bin>,
    from: Option<String>,
    to: Option<String>,
}

fn range(p: &RangeParams) -> Result<(Option<At>, Option<At>), Response> {
    let one = |s: &Option<String>| s.as_deref().map(At::parse).transpose().map_err(bad);
    Ok((one(&p.from)?, one(&p.to)?))
}

async fn density(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<RangeParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (from, to) = match range(&p) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let bin = p.bin.unwrap_or(Bin::Week);
    read(app, ws, move |s| s.density(from.as_ref(), to.as_ref(), bin)).await
}

async fn markers(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<RangeParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (from, to) = match range(&p) {
        Ok(r) => r,
        Err(e) => return e,
    };
    read_page(app, ws, move |s| {
        let (data, truncated) = s.markers(from.as_ref(), to.as_ref())?;
        Ok(Answer {
            data,
            next_cursor: None,
            truncated,
        })
    })
    .await
}

#[derive(Deserialize)]
struct NodeAt {
    node: Option<String>,
    at: Option<String>,
}

fn node_param(n: Option<String>) -> Result<String, Response> {
    n.filter(|n| !n.is_empty()).ok_or_else(|| {
        bad(QueryError::BadRequest {
            what: "node is missing".into(),
            why: "this route walks from a node".into(),
            action: "pass node=<id>".into(),
        })
    })
}

async fn impact_get(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<NodeAt>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (node, at) = match node_param(p.node).and_then(|n| Ok((n, at(p.at.as_deref())?))) {
        Ok(x) => x,
        Err(e) => return e,
    };
    read(app, ws, move |s| {
        s.impact(&ImpactInput::Nodes(vec![node]), &at)
    })
    .await
}

/// The request's one input, checked: exactly one of paths, ref, working_tree or since.
pub(crate) fn impact_input(r: ImpactRequest) -> Result<ImpactInput, QueryError> {
    let given = [
        r.paths.is_some(),
        r.git_ref.is_some(),
        r.working_tree.unwrap_or(false),
        r.since.is_some(),
    ];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(QueryError::BadRequest {
            what: "impact takes exactly one input".into(),
            why: "the input is paths, ref, working_tree: true or since".into(),
            action: "pass one of them".into(),
        });
    }
    Ok(match r {
        ImpactRequest { paths: Some(p), .. } => ImpactInput::Nodes(p),
        ImpactRequest {
            git_ref: Some(g), ..
        } => ImpactInput::Ref(g),
        ImpactRequest { since: Some(s), .. } => ImpactInput::Since(At::parse(&s)?),
        _ => ImpactInput::WorkingTree,
    })
}

async fn impact_post(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    bytes: Bytes,
) -> Response {
    let req: ImpactRequest = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let input = match impact_input(req) {
        Ok(i) => i,
        Err(e) => return bad(e),
    };
    read(app, ws, move |s| s.impact(&input, &At::Now)).await
}

#[derive(Deserialize)]
struct PathParams {
    from: Option<String>,
    to: Option<String>,
    max_hops: Option<usize>,
    at: Option<String>,
}

async fn path(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<PathParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let x = node_param(p.from).and_then(|f| Ok((f, node_param(p.to)?, at(p.at.as_deref())?)));
    let (from, to, at) = match x {
        Ok(x) => x,
        Err(e) => return e,
    };
    let hops = p.max_hops.unwrap_or(cartograph_query::MAX_DEPTH);
    read(app, ws, move |s| s.path(&from, &to, hops, &at)).await
}

#[derive(Deserialize)]
struct ReachParams {
    node: Option<String>,
    direction: Option<String>,
    depth: Option<usize>,
    at: Option<String>,
}

async fn reach(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<ReachParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (node, at) = match node_param(p.node).and_then(|n| Ok((n, at(p.at.as_deref())?))) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let (dir, depth) = (
        p.direction.unwrap_or_default(),
        p.depth.unwrap_or(cartograph_query::MAX_DEPTH),
    );
    read_page(app, ws, move |s| {
        let r = s.reach(&node, &dir, depth, &at)?;
        let truncated = r.truncated;
        Ok(Answer {
            data: r,
            next_cursor: None,
            truncated,
        })
    })
    .await
}

async fn stories(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<NodeAt>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (node, at) = match node_param(p.node).and_then(|n| Ok((n, at(p.at.as_deref())?))) {
        Ok(x) => x,
        Err(e) => return e,
    };
    read(app, ws, move |s| s.stories(&node, &at)).await
}

async fn mcp_tools() -> Response {
    crate::routes::ok(0, crate::mcp::registry_summary())
}

// ---------------------------------------------------------------- agents (SRV-21)

fn agent_error(e: &crate::agents::AgentError) -> Response {
    use crate::agents::AgentError as E;
    use cartograph_core::Explain;
    let code = match e {
        E::UnknownAgent(_) | E::NoWorkspace => ErrorCode::BadRequest,
        E::Changed { .. } => ErrorCode::PreconditionFailed,
        E::Malformed { .. } | E::Outside { .. } => ErrorCode::Unprocessable,
        E::Io { .. } => ErrorCode::Internal,
    };
    error(&ApiError::new(code, &e.what(), &e.why(), &e.action()))
}

fn no_home() -> Response {
    error(&ApiError::new(
        ErrorCode::Unprocessable,
        "the home folder isn't known",
        "agent config lives under your home folder",
        "set HOME and start Cartograph again",
    ))
}

/// The agents' user-level config: the server writes at user level only (F5); `--project` and
/// `--per-repo` stay in the CLI.
async fn agents(State(app): State<Arc<App>>) -> Response {
    let Some(home) = app.home.clone() else {
        return no_home();
    };
    let list = tokio::task::spawn_blocking(move || crate::agents::status(&home, None))
        .await
        .unwrap_or_default();
    crate::routes::ok(0, list)
}

async fn agents_change(app: Arc<App>, bytes: Bytes, write: bool) -> Response {
    let req: cartograph_api::types::AgentInstall = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let Some(home) = app.home.clone() else {
        return no_home();
    };
    let done = tokio::task::spawn_blocking(move || {
        if write {
            crate::agents::install(&home, None, &req)
        } else {
            crate::agents::preview(&home, None, &req)
        }
    })
    .await;
    match done {
        Ok(Ok(diffs)) => crate::routes::ok(0, diffs),
        Ok(Err(e)) => agent_error(&e),
        Err(_) => bad(QueryError::Internal),
    }
}

/// `POST /api/v1/agents/preview`: the diff of each file an install would change. Changes nothing.
async fn agents_preview(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    agents_change(app, bytes, false).await
}

/// `POST /api/v1/agents/install`: writes the agent's user config, after the preview (E3).
async fn agents_install(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    agents_change(app, bytes, true).await
}

// ---------------------------------------------------------------- /source

/// The most a `/source` answer holds.
const MAX_LINES: usize = 400;
const MAX_BYTES: usize = 64 * 1024;
/// The biggest blob read at all (the indexer's parse cap, B7).
const MAX_BLOB: u64 = 1 << 20;

#[derive(Deserialize)]
struct SourceParams {
    node: Option<String>,
    commit: Option<String>,
    lines: Option<String>,
}

/// `lines=a-b` (1-based, inclusive) or `lines=a`; the whole blob by default.
fn line_range(s: Option<&str>) -> Result<(usize, usize), QueryError> {
    let bad = || QueryError::BadRequest {
        what: "lines is malformed".into(),
        why: "lines is a 1-based line or range, such as 10-40".into(),
        action: "for example lines=10-40".into(),
    };
    let Some(s) = s.filter(|s| !s.is_empty()) else {
        return Ok((1, usize::MAX));
    };
    let (a, b) = s.split_once('-').unwrap_or((s, s));
    let (a, b): (usize, usize) = (
        a.trim().parse().map_err(|_| bad())?,
        b.trim().parse().map_err(|_| bad())?,
    );
    if a == 0 || b < a {
        return Err(bad());
    }
    Ok((a, b))
}

/// `GET …/source?node=&commit=&lines=`: text/plain with nosniff, at most 400 lines and 64 KB.
/// Binary and LFS blobs are refused.
async fn source(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<SourceParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let lines = match line_range(p.lines.as_deref()) {
        Ok(l) => l,
        Err(e) => return bad(e),
    };
    let node = match node_param(p.node) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let commit = match at(p.commit.as_deref()) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let done = tokio::task::spawn_blocking(move || -> Result<String, QueryError> {
        let store = app.store(&ws)?;
        let (repo, blob) = {
            let snap = store.snapshot(cartograph_query::Limits::default())?;
            snap.source_blob(&node, &commit)?
        };
        let refused = |what: &str, why: &str| QueryError::BadRequest {
            what: what.into(),
            why: why.into(),
            action: "open the file in your editor".into(),
        };
        let oid = cartograph_git::ObjectId::try_from(blob.as_slice())
            .map_err(|_| QueryError::Internal)?;
        let repo = cartograph_git::Repo::open(&repo).map_err(|e| {
            tracing::warn!(error = %e, "the repo behind an index can't be opened");
            QueryError::Unavailable {
                why: "the repository this file came from can't be opened".into(),
                action: "check that the repo is still on disk, then run cartograph doctor".into(),
            }
        })?;
        let bytes = cartograph_git::read_blob(&repo.local(), oid, MAX_BLOB).map_err(|_| {
            refused(
                "the file is too large to show",
                "files over 1 MB aren't read",
            )
        })?;
        if bytes.starts_with(b"version https://git-lfs.github.com/spec/") {
            return Err(refused(
                "the file is stored in Git LFS",
                "LFS pointers aren't read",
            ));
        }
        if bytes.iter().take(8000).any(|&b| b == 0) {
            return Err(refused("the file is binary", "only text is shown"));
        }
        let text = String::from_utf8_lossy(&bytes);
        let (a, b) = lines;
        let mut out = String::new();
        for line in text
            .split_inclusive('\n')
            .skip(a - 1)
            .take((b - a).saturating_add(1).min(MAX_LINES))
        {
            if out.len() + line.len() > MAX_BYTES {
                break;
            }
            out.push_str(line);
        }
        Ok(out)
    })
    .await;
    match done {
        Ok(Ok(text)) => {
            let mut res = Response::new(Body::from(text));
            *res.status_mut() = StatusCode::OK;
            let h = res.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            res
        }
        Ok(Err(e)) => bad(e),
        Err(_) => bad(QueryError::Internal),
    }
}

/// The Coverage view, Indexing and Services settings (IDX-20 to IDX-22): what the index can see,
/// how it was built and what it found. Each answers as of the last run.
async fn coverage(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    read(app, ws, |s| s.coverage()).await
}

async fn runs(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<ReadParams>, QueryRejection>,
) -> Response {
    let page = match params(q).and_then(|p| Page::parse(p.limit, p.cursor.as_deref()).map_err(bad))
    {
        Ok(p) => p,
        Err(e) => return e,
    };
    read_page(app, ws, move |s| s.runs(page).map(Answer::from)).await
}

async fn extractors(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    read(app, ws, |s| s.extractors()).await
}

async fn services(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    read(app, ws, |s| s.services()).await
}

/// The Contracts view (XS-08): specs, their operations and the drift between spec and code.
async fn contracts(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<ReadParams>, QueryRejection>,
) -> Response {
    let at = match params(q).and_then(|p| at(p.at.as_deref())) {
        Ok(a) => a,
        Err(e) => return e,
    };
    read(app, ws, move |s| s.contracts(&at)).await
}

/// Spec versions with their breaking changes and callers (XS-19).
async fn contract_versions(
    State(app): State<Arc<App>>,
    Path((ws, id)): Path<(String, String)>,
) -> Response {
    read(app, ws, move |s| s.contract_versions(&id)).await
}

#[derive(Deserialize)]
struct DiffParams {
    from: u32,
    to: u32,
}

/// The full diff between two versions of a spec, recomputed (XS-19).
async fn contract_diff(
    State(app): State<Arc<App>>,
    Path((ws, id)): Path<(String, String)>,
    q: Result<Query<DiffParams>, QueryRejection>,
) -> Response {
    let p = match params(q) {
        Ok(p) => p,
        Err(e) => return e,
    };
    read(app, ws, move |s| s.contract_diff(&id, p.from, p.to)).await
}

async fn environments(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    read(app, ws, |s| s.environments()).await
}

/// Owners from CODEOWNERS (XS-23): who owns how many files now.
async fn owners(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    read(app, ws, |s| s.owners()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ranges_parse_and_refuse_nonsense() {
        assert_eq!(line_range(None).unwrap(), (1, usize::MAX));
        assert_eq!(line_range(Some("7")).unwrap(), (7, 7));
        assert_eq!(line_range(Some("10-40")).unwrap(), (10, 40));
        for bad in ["0", "5-2", "a-b", "-", "1-", "../etc/passwd", "1;2"] {
            assert!(line_range(Some(bad)).is_err(), "{bad}");
        }
    }
}
