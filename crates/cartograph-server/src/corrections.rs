//! XS-13 on the wire: `GET`, `POST` and `DELETE /api/v1/w/{ws}/corrections` and the MCP `correct`
//! tool, over the one writer in `cartograph_index::corrections`.
//!
//! The file is the workspace's `cartograph.overrides.yaml`, found through the registry; no request
//! names a path. The guard has already checked the token, the Origin and the JSON type.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::response::Response;
use cartograph_api::types::{
    Actor, ApiError, Correction, CorrectionRequest, ErrorCode, FileDiff, JobKind,
};
use cartograph_discovery::overrides::{self, Entry};
use cartograph_index::corrections::{self, Why};
use cartograph_store::Index;
use serde::Serialize;
use serde_json::{Value, json};

use crate::live;
use crate::routes::{App, body, error, ok};
use crate::writes::blocking;

fn fail(code: ErrorCode, what: &str, why: &str, action: &str) -> ApiError {
    ApiError::new(code, what, why, action)
}

/// The override file and the index of `ws`, from the registry.
fn files(data: &Path, ws: &str) -> Result<(PathBuf, PathBuf), ApiError> {
    if crate::sync::manifest_path(data, ws).is_none() {
        return Err(fail(
            ErrorCode::NotFound,
            "no such workspace",
            "the registry has no workspace with this id",
            "list workspaces with GET /api/v1/workspaces",
        ));
    }
    let (w, _) = crate::sync::workspace_checked(data, ws)
        .map_err(|p| fail(ErrorCode::Unprocessable, &p.what, &p.why, &p.action))?;
    let file = w.overrides.clone().ok_or_else(|| {
        fail(
            ErrorCode::Unprocessable,
            "the workspace has no manifest folder",
            "the override file sits beside cartograph.yaml",
            "run cartograph init in the workspace folder",
        )
    })?;
    Ok((file, w.index_path()))
}

fn index(path: &Path) -> Result<Index, ApiError> {
    Index::open_read(path).map_err(|_| {
        fail(
            ErrorCode::Unprocessable,
            "the index isn't built yet",
            "a correction is checked against the links in the index",
            "wait for the first index to finish, then try again",
        )
    })
}

fn refused(r: corrections::Refused) -> ApiError {
    let code = match r.why {
        Why::Invalid => ErrorCode::Unprocessable,
        Why::Missing => ErrorCode::NotFound,
        Why::Io => ErrorCode::Internal,
    };
    fail(code, &r.what, &r.reason, &r.action)
}

fn file_diff(d: corrections::Diff) -> FileDiff {
    FileDiff {
        file: overrides::FILE_NAME.into(),
        diff: d.diff,
        applied: d.applied,
    }
}

/// An entry from the parts a request or a tool call gives. The wire enums spell the file's words.
pub(crate) fn entry(
    action: impl Serialize,
    key: impl Serialize,
    to: Option<String>,
    confidence: Option<impl Serialize>,
    reason: String,
) -> Result<Entry, ApiError> {
    let mut v = json!({"action": action, "key": key, "reason": reason});
    if let Some(k) = v["key"].as_object_mut() {
        k.retain(|_, x| !x.is_null());
    }
    if let Some(to) = to {
        v["to"] = Value::String(to);
    }
    if let Some(c) = confidence {
        v["confidence"] = json!(c);
    }
    serde_json::from_value(v).map_err(|e| {
        fail(
            ErrorCode::Unprocessable,
            "the correction isn't complete",
            &e.to_string(),
            "give each verb the fields it takes: see the schema of POST …/corrections",
        )
    })
}

/// Checks `entry` against the index and appends it to the workspace's override file (a dry run
/// only says how the file would change). A real write starts the sync that applies it.
pub(crate) fn add(
    data: &Path,
    ws: &str,
    entry: &Entry,
    dry_run: bool,
) -> Result<FileDiff, ApiError> {
    let (file, index_path) = files(data, ws)?;
    let ix = index(&index_path)?;
    corrections::append(&file, &ix.conn, entry, dry_run)
        .map(file_diff)
        .map_err(refused)
}

/// Removes entry `id` (`c<position>`).
pub(crate) fn undo(data: &Path, ws: &str, id: &str) -> Result<FileDiff, ApiError> {
    let (file, _) = files(data, ws)?;
    let at = id
        .strip_prefix('c')
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| {
            fail(
                ErrorCode::NotFound,
                "no such correction",
                "a correction's id is c and its position in the file",
                "list them with GET …/corrections",
            )
        })?;
    corrections::remove(&file, at, false)
        .map(file_diff)
        .map_err(refused)
}

/// The entries in file order, with the index's verdict on each.
fn list(data: &Path, ws: &str) -> Result<Vec<Correction>, ApiError> {
    let (file, index_path) = files(data, ws)?;
    let read = overrides::read(&file).map_err(|e| {
        fail(
            ErrorCode::Internal,
            "can't read cartograph.overrides.yaml",
            &e.to_string(),
            "check the file's permissions",
        )
    })?;
    let parsed = match read {
        None => return Ok(Vec::new()),
        Some((_, Ok(p))) => p,
        Some((_, Err(e))) => {
            return Err(fail(
                ErrorCode::Unprocessable,
                "cartograph.overrides.yaml doesn't read",
                &e.to_string(),
                &e.fix,
            ));
        }
    };
    // The index's rows match the file only while it has synced the file as it is.
    let rows = Index::open_read(&index_path)
        .ok()
        .and_then(|ix| corrections::rows(&ix.conn).ok())
        .filter(|r| r.len() == parsed.overrides.corrections.len());
    let mut out = Vec::new();
    for (i, e) in parsed.overrides.corrections.iter().enumerate() {
        let row = rows.as_ref().map(|r| &r[i]);
        let mut v = serde_json::to_value(e).expect("an entry serialises");
        v["id"] = json!(format!("c{i}"));
        v["by"] = json!(row.and_then(|r| r.by.clone()).unwrap_or_default());
        v["at"] = json!(
            row.and_then(|r| r.at)
                .map(cartograph_query::rfc3339)
                .unwrap_or_default()
        );
        v["stale"] = json!(row.is_some_and(|r| r.status != "bound"));
        out.push(serde_json::from_value(v).expect("an entry is a correction"));
    }
    Ok(out)
}

fn reply<T: Serialize>(r: Result<T, ApiError>) -> Response {
    match r {
        Ok(v) => ok(0, v),
        Err(e) => error(&e),
    }
}

/// `GET /api/v1/w/{ws}/corrections`.
pub async fn get(State(app): State<Arc<App>>, UrlPath(ws): UrlPath<String>) -> Response {
    blocking(move || reply(list(&app.data_dir, &ws))).await
}

/// `POST /api/v1/w/{ws}/corrections`: appends an entry, or with `dry_run` returns the diff it
/// would make. The write starts a sync, whose events carry the result to the app.
pub async fn post(
    State(app): State<Arc<App>>,
    UrlPath(ws): UrlPath<String>,
    bytes: Bytes,
) -> Response {
    let req: CorrectionRequest = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    blocking(move || {
        let done = entry(req.action, req.key, req.to, req.confidence, req.reason)
            .and_then(|e| add(&app.data_dir, &ws, &e, req.dry_run));
        if done.as_ref().is_ok_and(|d| d.applied) {
            live::start(&app, &ws, JobKind::Sync, Actor::App, false);
        }
        reply(done)
    })
    .await
}

/// `DELETE /api/v1/w/{ws}/corrections/{id}`: undo; the sync restores the link.
pub async fn delete(
    State(app): State<Arc<App>>,
    UrlPath((ws, id)): UrlPath<(String, String)>,
) -> Response {
    blocking(move || {
        let done = undo(&app.data_dir, &ws, &id);
        if done.is_ok() {
            live::start(&app, &ws, JobKind::Sync, Actor::App, false);
        }
        reply(done)
    })
    .await
}
