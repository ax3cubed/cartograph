//! XS-22 on the wire: `GET /api/v1/w/{ws}/rules` and `…/rules/violations`, over the one evaluation
//! in `cartograph_index::rules`. The rules are the manifest's and the baseline is the file beside
//! it; no request names a path.

use std::path::Path;
use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use cartograph_api::types::{
    ApiError, ConfidenceLevel, Envelope, ErrorCode, Rule, RuleCommit, RuleViolation,
};
use cartograph_index::rules::{self, Report};
use cartograph_query::{At, Limits, Page, Paged, QueryError};
use serde::{Deserialize, Serialize};

use crate::routes::{App, error, json, ok};
use crate::writes::blocking;

fn report(data: &Path, ws: &str) -> Result<Report, ApiError> {
    let Some(manifest) = crate::sync::manifest_path(data, ws) else {
        // An index nobody registered (`cartograph index`) has no manifest, so no rules.
        let indexed = cartograph_core::paths::workspace_dir(data, ws)
            .join("index.cartograph")
            .is_file();
        return indexed.then(Report::default).ok_or_else(|| {
            ApiError::new(
                ErrorCode::NotFound,
                "no such workspace",
                "the registry has no workspace with this id",
                "list workspaces with GET /api/v1/workspaces",
            )
        });
    };
    let (w, _) = crate::sync::workspace_checked(data, ws)
        .map_err(|p| ApiError::new(ErrorCode::Unprocessable, &p.what, &p.why, &p.action))?;
    rules::report(&manifest, &w.index_path()).map_err(|(what, why, action)| {
        ApiError::new(ErrorCode::Unprocessable, &what, &why, &action)
    })
}

fn reply<T: Serialize>(r: Result<T, ApiError>) -> Response {
    match r {
        Ok(v) => ok(0, v),
        Err(e) => error(&e),
    }
}

fn rules_of(r: &Report) -> Vec<Rule> {
    r.rules
        .iter()
        .map(|x| Rule {
            id: x.rule.name.clone(),
            kind: x.rule.kind.as_str().into(),
            severity: x.rule.severity.as_str().into(),
            description: x.rule.reason.clone().unwrap_or_default(),
            violations: x.violations as u32,
            baselined: x.baselined as u32,
        })
        .collect()
}

fn violations_of(r: &Report) -> Vec<RuleViolation> {
    let seq = |n: i64| u32::try_from(n).unwrap_or(0);
    r.found
        .iter()
        .map(|f| {
            let v = &f.violation;
            RuleViolation {
                rule: v.rule.clone(),
                severity: f.severity.as_str().into(),
                kind: v.kind.clone(),
                from: v.from.clone(),
                to: v.to.clone(),
                op: (!v.op.is_empty()).then(|| v.op.clone()),
                level: match v.level.as_deref() {
                    Some("exact") => Some(ConfidenceLevel::Exact),
                    Some("likely") => Some(ConfidenceLevel::Likely),
                    _ => None,
                },
                baselined: f.baselined,
                live: v.live,
                intervals: v
                    .intervals
                    .iter()
                    .map(|&(a, b)| (seq(a), b.map(seq)))
                    .collect(),
            }
        })
        .collect()
}

/// `GET /api/v1/w/{ws}/rules`.
pub async fn get(State(app): State<Arc<App>>, UrlPath(ws): UrlPath<String>) -> Response {
    blocking(move || reply(report(&app.data_dir, &ws).map(|r| rules_of(&r)))).await
}

#[derive(Deserialize)]
pub struct PageParams {
    limit: Option<u32>,
    cursor: Option<String>,
    rule: Option<String>,
}

fn page_of<T: Serialize>(p: Paged<T>) -> Response {
    let mut env = Envelope::new(0, p.rows);
    env.next_cursor = p.next_cursor;
    json(StatusCode::OK, &env)
}

fn bad_query(e: &QueryError) -> Response {
    error(&e.to_api())
}

/// The page asked for, or the 400 that says why not.
fn page_param(
    q: Result<Query<PageParams>, QueryRejection>,
) -> Result<(Page, PageParams), Response> {
    let Ok(Query(p)) = q else {
        return Err(error(&ApiError::new(
            ErrorCode::BadRequest,
            "the query string is malformed",
            "limit is a number and cursor is the next_cursor of an earlier answer",
            "check the parameter names and values against the API reference",
        )));
    };
    let page = Page::parse(p.limit, p.cursor.as_deref()).map_err(|e| bad_query(&e))?;
    Ok((page, p))
}

/// `GET /api/v1/w/{ws}/rules/violations`: the violations, live ones first, one page at a time,
/// with the steps each held over. `rule` names one rule.
pub async fn violations(
    State(app): State<Arc<App>>,
    UrlPath(ws): UrlPath<String>,
    q: Result<Query<PageParams>, QueryRejection>,
) -> Response {
    let (page, p) = match page_param(q) {
        Ok(x) => x,
        Err(e) => return e,
    };
    blocking(move || match report(&app.data_dir, &ws) {
        Ok(r) => {
            let mut all = violations_of(&r);
            if let Some(rule) = p.rule.as_deref().filter(|r| !r.is_empty()) {
                all.retain(|v| v.rule == rule);
            }
            all.sort_by_key(|v| std::cmp::Reverse(v.live));
            page_of(page.take(all))
        }
        Err(e) => error(&e),
    })
    .await
}

/// `GET /api/v1/w/{ws}/rules/commits`: each commit, newest first, with the violations that held
/// after it. A violation holds over the steps `[from, to)` of its intervals.
pub async fn commits(
    State(app): State<Arc<App>>,
    UrlPath(ws): UrlPath<String>,
    q: Result<Query<PageParams>, QueryRejection>,
) -> Response {
    let (page, _) = match page_param(q) {
        Ok(x) => x,
        Err(e) => return e,
    };
    blocking(move || {
        let found = match report(&app.data_dir, &ws) {
            Ok(r) => violations_of(&r),
            Err(e) => return error(&e),
        };
        let read = || -> Result<Paged<RuleCommit>, QueryError> {
            let store = app.store(&ws)?;
            let snap = store.snapshot(Limits::default())?;
            let list = snap.commits(&At::Now, page)?;
            let holds = |v: &RuleViolation, seq: u32| {
                v.intervals
                    .iter()
                    .any(|&(a, b)| a <= seq && b.is_none_or(|b| seq < b))
            };
            Ok(Paged {
                rows: list
                    .rows
                    .into_iter()
                    .map(|commit| {
                        let seq = commit.seq as u32;
                        let held: Vec<_> = found.iter().filter(|v| holds(v, seq)).collect();
                        RuleCommit {
                            failing: held
                                .iter()
                                .filter(|v| v.severity == "error" && !v.baselined)
                                .count() as u32,
                            violations: held.len() as u32,
                            commit,
                        }
                    })
                    .collect(),
                next_cursor: list.next_cursor,
            })
        };
        match read() {
            Ok(p) => page_of(p),
            Err(e) => bad_query(&e),
        }
    })
    .await
}
