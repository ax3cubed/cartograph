//! XS-22 on the wire: `GET /api/v1/w/{ws}/rules` and `…/rules/violations`, over the one evaluation
//! in `cartograph_index::rules`. The rules are the manifest's and the baseline is the file beside
//! it; no request names a path.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::response::Response;
use cartograph_api::types::{ApiError, ConfidenceLevel, ErrorCode, Rule, RuleViolation};
use cartograph_index::rules::{self, Report};
use serde::Serialize;

use crate::routes::{App, error, ok};
use crate::writes::blocking;

fn report(data: &Path, ws: &str) -> Result<Report, ApiError> {
    let manifest = crate::sync::manifest_path(data, ws).ok_or_else(|| {
        ApiError::new(
            ErrorCode::NotFound,
            "no such workspace",
            "the registry has no workspace with this id",
            "list workspaces with GET /api/v1/workspaces",
        )
    })?;
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

/// `GET /api/v1/w/{ws}/rules/violations`: every violation, with the steps it held over.
pub async fn violations(State(app): State<Arc<App>>, UrlPath(ws): UrlPath<String>) -> Response {
    blocking(move || reply(report(&app.data_dir, &ws).map(|r| violations_of(&r)))).await
}
