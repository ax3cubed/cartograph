//! `cartograph status` (SRV-03): each repo's indexed commit, its HEAD and how far behind it is,
//! from IDX-15's check (ref reads only, no network). `--check` exits 3 when anything is behind;
//! `--line` is one line of counts and fixed words for a SessionStart hook (F8: no repo or branch
//! names, since it is loaded into every agent session).

use std::path::Path;

use cartograph_core::{ExitCode, Report};
use cartograph_discovery::setup;
use cartograph_index::derive::{CoverageReport, coverage_report};
use cartograph_index::freshness::{FreshnessReport, RepoFreshness, State, freshness};
use serde_json::{Value, json};

use crate::data_dir;
use crate::output::Out;

const NO_WORKSPACE: &str = "cartograph: no workspace here; run `cartograph init`";

pub fn run(out: &Out, dir: &Path, check: bool, line: bool, unresolved: bool) -> Result<(), Report> {
    let report = match data_dir().and_then(|data| find(&data, dir)) {
        Ok(r) => r,
        // The hook runs in every session, in any folder: it never fails.
        Err(_) if line => {
            out.print(NO_WORKSPACE, json!({ "state": "none" }));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if line {
        out.print(&one_line(&report.0), json!({ "line": one_line(&report.0) }));
        return Ok(());
    }
    let (report, not_added, cov) = report;
    let mut text = human(&report);
    if !not_added.is_empty() {
        text += &format!(
            "\nnot added: {} (run `cartograph init` to sync them)",
            not_added.join(", ")
        );
    }
    let mut j = to_json(&report);
    j["not_added"] = json!(not_added);
    if let Some(c) = &cov {
        text += &format!("\n{}", coverage_line(c));
        if unresolved {
            for (path, line, spec) in &c.unresolved {
                text += &format!("\n  {path}:{line}  {spec}");
            }
            if c.dropped > 0 {
                text += &format!("\n  and {} more, not kept", c.dropped);
            }
        }
        j["coverage"] = json!({
            "files_total": c.files_total,
            "files_parsed": c.files_parsed,
            "imports_total": c.imports_total,
            "imports_resolved": c.imports_resolved,
            "unresolved_count": c.unresolved.len() as i64 + c.dropped,
        });
        if unresolved {
            // The specifier is repository text: untrusted (F8).
            j["coverage"]["unresolved"] = c
                .unresolved
                .iter()
                .map(|(path, line, spec)| json!({ "path": path, "line": line, "untrusted_text": spec }))
                .collect();
        }
    }
    out.print(&text, j);
    if check && report.behind() {
        let n = report
            .repos
            .iter()
            .filter(|r| r.behind > 0 || r.rewritten || r.unindexed())
            .count();
        return Err(Report {
            what: format!("{n} of {} repos are behind the index", report.repos.len()),
            why: "commits landed since the last sync".into(),
            action: "run `cartograph sync`".into(),
            exit_code: ExitCode::Behind,
        });
    }
    Ok(())
}

/// "files parsed x / y, imports resolved z%" (IDX-22).
fn coverage_line(c: &CoverageReport) -> String {
    let pct = if c.imports_total == 0 {
        100.0
    } else {
        c.imports_resolved as f64 * 100.0 / c.imports_total as f64
    };
    format!(
        "files parsed {} / {}, imports resolved {pct:.1}% ({} of {})",
        c.files_parsed, c.files_total, c.imports_resolved, c.imports_total
    )
}

/// The report, the manifest repos left out of sync because they aren't registered (F8), and the
/// index's coverage when it has one.
fn find(
    data: &Path,
    dir: &Path,
) -> Result<(FreshnessReport, Vec<String>, Option<CoverageReport>), Report> {
    let applied = setup::locate(data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    let (ws, not_added) = cartograph_server::workspace_checked(data, &applied.workspace_id)
        .map_err(|p| Report {
            what: p.what,
            why: p.why,
            action: p.action,
            exit_code: ExitCode::Error,
        })?;
    let cov = cartograph_store::Index::open_read(&ws.index_path())
        .ok()
        .and_then(|ix| coverage_report(&ix.conn).ok().flatten());
    Ok((freshness(&ws), not_added, cov))
}

fn behind(r: &RepoFreshness) -> String {
    if r.behind_capped {
        format!("{}+", r.behind)
    } else {
        r.behind.to_string()
    }
}

fn short<T: std::fmt::Display>(o: Option<T>) -> String {
    o.map_or_else(|| "-".into(), |o| o.to_string()[..10].to_owned())
}

fn human(r: &FreshnessReport) -> String {
    let mut s = format!("state {}", r.state.as_str());
    if r.syncing {
        s.push_str(" (a sync is running)");
    }
    for x in &r.repos {
        let mut notes = Vec::new();
        if x.rewritten {
            notes.push("history rewritten");
        }
        if x.shallow {
            notes.push("shallow");
        }
        if x.dirty == Some(true) {
            notes.push("uncommitted changes");
        }
        s.push_str(&format!(
            "\n{}  indexed {}  head {}  behind {}{}",
            x.name,
            short(x.indexed),
            short(x.head),
            behind(x),
            if notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", notes.join(", "))
            }
        ));
    }
    s
}

fn to_json(r: &FreshnessReport) -> Value {
    json!({
        "state": r.state.as_str(),
        "syncing": r.syncing,
        "behind": r.repos.iter().map(|x| u64::from(x.behind)).sum::<u64>(),
        "repos": r.repos.iter().map(|x| json!({
            "name": x.name,
            "indexed_commit": x.indexed.map(|o| o.to_string()),
            "head": x.head.map(|o| o.to_string()),
            "behind": x.behind,
            "behind_capped": x.behind_capped,
            "rewritten": x.rewritten,
            "shallow": x.shallow,
            "dirty": x.dirty,
        })).collect::<Vec<_>>(),
    })
}

/// Counts and fixed words only, under 120 characters.
fn one_line(r: &FreshnessReport) -> String {
    let n = r.repos.len();
    let behind: Vec<&RepoFreshness> = r
        .repos
        .iter()
        .filter(|x| x.behind > 0 || x.rewritten)
        .collect();
    let commits: u64 = behind.iter().map(|x| u64::from(x.behind)).sum();
    let capped = behind.iter().any(|x| x.behind_capped);
    let mut s = match r.state {
        State::None => "cartograph: not indexed yet; run `cartograph sync`".to_owned(),
        _ if behind.is_empty() => format!("cartograph: index {}, {n} repos", r.state.as_str()),
        _ => format!(
            "cartograph: {commits}{} commits behind in {} of {n} repos",
            if capped { "+" } else { "" },
            behind.len()
        ),
    };
    if r.syncing {
        s.push_str("; syncing");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(behind: u32, capped: bool) -> RepoFreshness {
        RepoFreshness {
            name: "secret-repo-name".into(),
            indexed: None,
            head: None,
            behind,
            behind_capped: capped,
            rewritten: false,
            shallow: false,
            on_branch_other_than_indexed: false,
            dirty: None,
        }
    }

    #[test]
    fn the_line_is_short_and_names_nothing() {
        let r = FreshnessReport {
            state: State::Behind,
            syncing: true,
            dirty_flag: false,
            repos: vec![repo(1000, true), repo(0, false), repo(3, false)],
        };
        let l = one_line(&r);
        assert_eq!(
            l,
            "cartograph: 1003+ commits behind in 2 of 3 repos; syncing"
        );
        assert!(l.len() < 120 && !l.contains("secret"));
        let current = FreshnessReport {
            state: State::Current,
            syncing: false,
            dirty_flag: false,
            repos: vec![repo(0, false)],
        };
        assert_eq!(one_line(&current), "cartograph: index current, 1 repos");
        assert!(NO_WORKSPACE.len() < 120);
    }
}
