//! The routes: reads (SRV-05) are thin wrappers over `cartograph-query`, run on the blocking pool
//! and answered in the shared envelope; events and jobs are in `live` (SRV-13), writes in `writes`
//! (SRV-14), and every other path is the embedded app (SRV-06).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use axum::routing::{get, post};
use cartograph_api::types::{ApiError, Envelope, ErrorBody, ErrorCode, GraphParams, Meta, Version};
use cartograph_query::{GraphQuery, Limits, QueryError, Snapshot, Store, valid_workspace_id};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::ServerInfo;
use crate::guard::{CONCURRENCY, guard};
use crate::live::{self, Live};
use crate::writes;

pub struct App {
    pub(crate) data_dir: PathBuf,
    /// The folder `fs/list` may browse: the user's home.
    pub(crate) home: Option<PathBuf>,
    pub(crate) live: Live,
    pub(crate) token: String,
    pub(crate) origin: String,
    /// The bound address (`127.0.0.1:<port>` by default), `localhost:<port>`, `[::1]:<port>`.
    pub(crate) hosts: [String; 3],
    pub(crate) slots: Arc<Semaphore>,
    stores: Mutex<HashMap<String, Arc<Store>>>,
}

impl App {
    pub fn new(data_dir: PathBuf, home: Option<PathBuf>, info: &ServerInfo) -> Self {
        let p = info.port;
        Self {
            data_dir,
            home,
            live: Live::default(),
            token: info.token.clone(),
            origin: info.origin(),
            // The bound address first: --host takes any 127.x, and the origin and the printed URL
            // use it, so a fixed 127.0.0.1 would refuse every request to 127.0.0.2.
            hosts: [
                std::net::SocketAddr::new(info.host, p).to_string(),
                format!("localhost:{p}"),
                format!("[::1]:{p}"),
            ],
            slots: Arc::new(Semaphore::new(CONCURRENCY)),
            stores: Mutex::new(HashMap::new()),
        }
    }

    /// One store per workspace, opened on first use and kept; it reopens itself after a rebuild.
    pub(crate) fn store(&self, ws: &str) -> Result<Arc<Store>, QueryError> {
        // A read racing a workspace's removal can re-cache it; the file check drops that entry.
        let index =
            cartograph_core::paths::workspace_dir(&self.data_dir, ws).join("index.cartograph");
        let mut stores = self.stores.lock().unwrap_or_else(|p| p.into_inner());
        match stores.get(ws) {
            Some(s) if index.exists() => return Ok(s.clone()),
            Some(_) => {
                stores.remove(ws);
            }
            None => {}
        }
        drop(stores);
        let s = Arc::new(Store::open(&self.data_dir, ws)?);
        self.stores
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(ws.to_owned(), s.clone());
        Ok(s)
    }

    /// Drops the cached store of a workspace that was removed.
    pub(crate) fn forget_store(&self, ws: &str) {
        self.stores
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(ws);
    }

    /// A workspace whose index file exists but can't be opened, as it is listed instead of being
    /// dropped: `needs_migrate` for an older format (a migration or rebuild is on its way, see
    /// [`live::upgrade_all`]), `failed` otherwise, with the reason. `None` without an index file.
    pub(crate) fn unopened(
        &self,
        ws: &str,
        why: &QueryError,
    ) -> Option<(
        cartograph_api::types::WorkspaceSummary,
        cartograph_api::types::Freshness,
    )> {
        use cartograph_api::types::{Freshness, IndexState, Problem, WorkspaceSummary};
        use cartograph_core::Explain;
        if !valid_workspace_id(ws) {
            return None;
        }
        let index =
            cartograph_core::paths::workspace_dir(&self.data_dir, ws).join("index.cartograph");
        if !index.is_file() {
            return None;
        }
        let old = cartograph_store::Index::open_read(&index)
            .is_ok_and(|ix| ix.state == cartograph_store::State::NeedsMigrate);
        let (state, problem) = if old {
            (
                IndexState::NeedsMigrate,
                Problem {
                    what: "the index was written by an older Cartograph".into(),
                    why: "its format is migrated, or rebuilt from git, before it is read".into(),
                    action: "wait for the sync to finish, or run `cartograph sync`".into(),
                },
            )
        } else {
            (
                IndexState::Failed,
                Problem {
                    what: why.what(),
                    why: why.why(),
                    action: why.action(),
                },
            )
        };
        let registered = cartograph_discovery::Registry::load(&self.data_dir)
            .ok()
            .and_then(|(r, _)| r.workspaces.into_iter().find(|w| w.id == ws));
        let summary = WorkspaceSummary {
            id: ws.to_owned(),
            name: registered
                .as_ref()
                .map_or_else(|| ws.to_owned(), |w| w.name.clone()),
            path: registered
                .as_ref()
                .and_then(|w| {
                    w.manifest
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                })
                .unwrap_or_default(),
            demo: false,
            repos: 0,
            services: 0,
            modules: 0,
            files: 0,
            commits: 0,
            state,
            built_at: None,
        };
        let fresh = Freshness {
            state,
            format_version: cartograph_query::FORMAT_VERSION as u32,
            built_at: None,
            behind: 0,
            repos: vec![],
            progress: None,
            error: Some(problem),
        };
        Some((summary, fresh))
    }

    /// Workspace ids with an index file, sorted: the registry's (IDX-11), plus any index folder a
    /// single-repo `cartograph index` wrote without one.
    pub(crate) fn workspace_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(self.data_dir.join("workspaces"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .chain(crate::sync::workspace_ids(&self.data_dir))
            .filter(|id| valid_workspace_id(id))
            .filter(|id| {
                cartograph_core::paths::workspace_dir(&self.data_dir, id)
                    .join("index.cartograph")
                    .is_file()
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/api/version", get(version))
        .route("/api/v1/meta", get(meta))
        .route(
            "/api/v1/workspaces",
            get(workspaces).post(writes::create_workspace),
        )
        .route("/api/v1/workspaces/{ws}", get(workspace))
        .route("/api/v1/w/{ws}/freshness", get(freshness))
        .route("/api/v1/w/{ws}/repos", get(repos))
        .route("/api/v1/w/{ws}/graph", get(graph))
        .route("/api/v1/events", get(live::events))
        .route("/api/v1/w/{ws}/jobs", post(live::post_job))
        .route(
            "/api/v1/jobs/{id}",
            get(live::get_job).delete(live::cancel_job),
        )
        .route(
            "/api/v1/prefs",
            get(writes::get_prefs).put(writes::put_prefs),
        )
        .route("/api/v1/fs/list", get(writes::fs_list))
        .route("/api/v1/discover", post(writes::discover))
        .route(
            "/api/v1/w/{ws}/config",
            get(writes::get_config).put(writes::put_config),
        )
        .route(
            "/api/v1/w/{ws}/config/preview",
            post(writes::preview_config),
        )
        .fallback(fallback)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(axum::middleware::from_fn_with_state(app.clone(), guard))
        .layer(axum::extract::DefaultBodyLimit::max(1 << 20))
        .with_state(app)
}

/// The error body with its status. Never SQL, a path outside the workspace or a stack trace:
/// every message comes from a fixed string or a `QueryError` that holds none.
pub fn error(e: &ApiError) -> Response {
    let mut res = json(
        StatusCode::from_u16(e.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        &ErrorBody::from(e.clone()),
    );
    // A timeout is the server's state, not the request's fault: say when to try again.
    if e.code == ErrorCode::Timeout {
        res.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    }
    res
}

pub(crate) fn json<T: Serialize>(status: StatusCode, body: &T) -> Response {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let mut res = Response::new(Body::from(bytes));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

pub(crate) fn ok<T: Serialize>(generation: u64, data: T) -> Response {
    json(StatusCode::OK, &Envelope::new(generation, data))
}

/// Parses a write's JSON body against its closed schema: an unknown field is a 400, like a
/// missing one.
pub(crate) fn body<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Response> {
    serde_json::from_slice(bytes).map_err(|e| {
        error(&ApiError::new(
            ErrorCode::BadRequest,
            "the request body doesn't match the schema",
            &e.to_string(),
            "check the body against the API reference; unknown fields are refused",
        ))
    })
}

/// Runs `f` on one snapshot of workspace `ws`, off the async threads, and answers with the
/// envelope and the `Cartograph-Generation` and `Cartograph-State` headers.
async fn read<T, F>(app: Arc<App>, ws: String, f: F) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce(&Snapshot<'_>) -> Result<T, QueryError> + Send + 'static,
{
    let done = tokio::task::spawn_blocking(move || {
        let store = app.store(&ws)?;
        let snap = store.snapshot(Limits::default())?;
        let state = snap.index_state()?;
        let data = f(&snap)?;
        // D1: every read checks freshness, and a workspace that is behind starts a sync.
        if let Ok(fresh) = crate::sync::freshness(&app.data_dir, &ws, &snap) {
            app.ensure_fresh(&ws, &fresh);
        }
        Ok::<_, QueryError>((snap.generation, state, data))
    })
    .await;
    match done {
        Ok(Ok((generation, state, data))) => {
            let mut res = ok(generation, data);
            let h = res.headers_mut();
            h.insert("cartograph-generation", HeaderValue::from(generation));
            let state = serde_json::to_value(state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            if let Ok(v) = HeaderValue::from_str(&state) {
                h.insert("cartograph-state", v);
            }
            res
        }
        Ok(Err(e)) => error(&e.to_api()),
        // A panic in a query: the details go to the log, never to the body.
        Err(e) => {
            tracing::error!(error = %e, "a query panicked");
            error(&QueryError::Internal.to_api())
        }
    }
}

async fn version() -> Response {
    ok(
        0,
        Version {
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )
}

async fn meta() -> Response {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    ok(
        0,
        Meta {
            version: env!("CARGO_PKG_VERSION").into(),
            format_version: cartograph_query::FORMAT_VERSION as u32,
            now: cartograph_query::rfc3339(now),
            schema: cartograph_api::SCHEMA_VERSION,
        },
    )
}

/// The workspace's own name and root folder (its manifest's) from the registry, over the summary's
/// first-repo stand-ins. A single-repo index with no registry entry keeps those.
fn registered(
    reg: Option<&cartograph_discovery::Registry>,
    mut s: cartograph_api::types::WorkspaceSummary,
) -> cartograph_api::types::WorkspaceSummary {
    if let Some(w) = reg.and_then(|r| r.workspaces.iter().find(|w| w.id == s.id)) {
        s.name = w.name.clone();
        if let Some(root) = w.manifest.parent() {
            s.path = root.display().to_string();
        }
    }
    s
}

async fn workspaces(State(app): State<Arc<App>>) -> Response {
    let listed = tokio::task::spawn_blocking(move || {
        let reg = cartograph_discovery::Registry::load(&app.data_dir)
            .ok()
            .map(|(r, _)| r);
        app.workspace_ids()
            .into_iter()
            .filter_map(|id| {
                let summary = app
                    .store(&id)
                    .and_then(|store| store.snapshot(Limits::default())?.workspace_summary(&id));
                match summary {
                    Ok(s) => Some(registered(reg.as_ref(), s)),
                    // Listed with its state rather than dropped.
                    Err(e) => app.unopened(&id, &e).map(|(s, _)| s),
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    ok(0, listed)
}

async fn workspace(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    if let Some(r) = unopened(&app, &ws, |(s, _)| ok(0, s)).await {
        return r;
    }
    let id = ws.clone();
    let data = app.data_dir.clone();
    read(app, ws, move |s| {
        let reg = cartograph_discovery::Registry::load(&data)
            .ok()
            .map(|(r, _)| r);
        s.workspace_summary(&id)
            .map(|w| registered(reg.as_ref(), w))
    })
    .await
}

async fn freshness(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    if let Some(r) = unopened(&app, &ws, |(_, f)| ok(0, f)).await {
        return r;
    }
    let data = app.data_dir.clone();
    let id = ws.clone();
    read(app, ws, move |s| crate::sync::freshness(&data, &id, s)).await
}

/// The answer for a workspace whose index exists but can't be opened, if that is the case.
async fn unopened(
    app: &Arc<App>,
    ws: &str,
    answer: impl FnOnce(
        (
            cartograph_api::types::WorkspaceSummary,
            cartograph_api::types::Freshness,
        ),
    ) -> Response,
) -> Option<Response> {
    let (a, id) = (app.clone(), ws.to_owned());
    // The store's state is known without a snapshot, so a readable index costs nothing more.
    let found = tokio::task::spawn_blocking(move || match a.store(&id) {
        Ok(s) if s.state() != cartograph_store::State::NeedsMigrate => None,
        Ok(_) => a.unopened(
            &id,
            &QueryError::Unavailable {
                why: String::new(),
                action: String::new(),
            },
        ),
        Err(e) => a.unopened(&id, &e),
    })
    .await
    .ok()
    .flatten()?;
    Some(answer(found))
}

async fn repos(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    let data = app.data_dir.clone();
    let id = ws.clone();
    read(app, ws, move |s| crate::sync::repos(&data, &id, s)).await
}

async fn graph(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    q: Result<Query<GraphParams>, QueryRejection>,
) -> Response {
    let Query(p) = match q {
        Ok(q) => q,
        Err(r) => {
            return error(&ApiError::new(
                ErrorCode::BadRequest,
                "the query string is malformed",
                &r.body_text(),
                "check the parameter names and values against the API reference",
            ));
        }
    };
    read(app, ws, move |s| s.graph(&GraphQuery::from_params(&p)?)).await
}

/// `/api/…` that matched no route gets the JSON 404; any other path is the app (SRV-06).
async fn fallback(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if uri.path().starts_with("/api/") || uri.path() == "/api" {
        return not_found().await;
    }
    if method != Method::GET && method != Method::HEAD {
        return method_not_allowed().await;
    }
    crate::ui::serve(uri.path(), &headers, method == Method::HEAD)
}

async fn not_found() -> Response {
    error(&ApiError::new(
        ErrorCode::NotFound,
        "no such route",
        "the API lives under /api/v1",
        "see GET /api/v1/meta for the schema version",
    ))
}

async fn method_not_allowed() -> Response {
    error(&ApiError::new(
        ErrorCode::MethodNotAllowed,
        "this route doesn't take that method",
        "the routes served so far are reads",
        "use GET",
    ))
}
