//! `cartograph check` (XS-12): fails on a correction that matches nothing. Rule violations join it
//! when rules land; until then `rules` is empty.
//!
//! The statuses are the index's, written by the last sync. The file is read here too, so one that
//! doesn't parse fails with its line, and so does an index that is behind the file.

use std::path::Path;

use cartograph_core::{ExitCode, Report};
use cartograph_discovery::overrides::{self, Stale};
use cartograph_discovery::setup;
use cartograph_index::corrections::{self, Row};
use serde_json::json;

use crate::data_dir;
use crate::output::{Out, sanitize};

fn fail(what: String, why: String, action: &str) -> Report {
    Report {
        what,
        why,
        action: action.into(),
        exit_code: ExitCode::Error,
    }
}

pub fn run(out: &Out, dir: &Path) -> Result<(), Report> {
    let data = data_dir()?;
    let applied = setup::locate(&data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    let (ws, _) = cartograph_server::workspace_checked(&data, &applied.workspace_id)
        .map_err(|p| fail(p.what, p.why, &p.action))?;
    let file = ws.overrides.clone().ok_or_else(|| {
        fail(
            "the workspace has no manifest folder".into(),
            String::new(),
            "run `cartograph init`",
        )
    })?;
    let name = overrides::FILE_NAME;
    let (entries, mode) = match overrides::read(&file) {
        Ok(None) => (0, Stale::Error),
        Ok(Some((_, Err(e)))) => {
            return Err(fail(format!("{name} doesn't read"), e.to_string(), &e.fix));
        }
        Ok(Some((_, Ok(p)))) => (p.overrides.corrections.len(), p.overrides.stale),
        Err(e) => {
            return Err(fail(
                format!("can't read {name}"),
                e.to_string(),
                "check the file's permissions",
            ));
        }
    };
    let rows: Vec<Row> = match cartograph_store::Index::open_read(&ws.index_path()) {
        Ok(ix) => corrections::rows(&ix.conn).map_err(|e| {
            fail(
                "can't read the index".into(),
                e.to_string(),
                "run `cartograph sync`",
            )
        })?,
        Err(_) if entries == 0 => Vec::new(),
        Err(e) => {
            return Err(fail(
                "can't open the index".into(),
                e.to_string(),
                "run `cartograph sync`",
            ));
        }
    };
    if rows.len() != entries {
        return Err(fail(
            format!(
                "the index has {} of the {entries} corrections in {name}",
                rows.len()
            ),
            "the file changed since the last sync".into(),
            "run `cartograph sync`",
        ));
    }

    let line = |r: &Row| {
        format!(
            "{name}:{} {} {} {}: {}",
            r.line,
            r.status,
            r.action,
            r.kind,
            sanitize(&r.key)
        )
    };
    let of = |s: &str| {
        rows.iter()
            .filter(|r| r.status == s)
            .map(line)
            .collect::<Vec<_>>()
    };
    let (stale, ambiguous) = (of("stale"), of("ambiguous"));
    let human = if rows.is_empty() {
        "no corrections".to_owned()
    } else {
        format!(
            "{} corrections: {} bound, {} stale, {} ambiguous",
            rows.len(),
            rows.len() - stale.len() - ambiguous.len(),
            stale.len(),
            ambiguous.len()
        )
    };
    let corrections_json: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "line": r.line, "action": r.action, "kind": r.kind, "key": r.key,
                "status": r.status, "links": r.links, "by": r.by, "at": r.at,
                // The reason is repo text: untrusted (F8).
                "untrusted_text": r.reason,
            })
        })
        .collect();
    out.print(
        &human,
        json!({ "corrections": corrections_json, "rules": [] }),
    );

    // An entry that can't be applied is said once, on stderr; `stale: off` keeps quiet about
    // the stale ones.
    for l in &ambiguous {
        eprintln!("warning: {}", sanitize(l));
    }
    if mode == Stale::Off || stale.is_empty() {
        return Ok(());
    }
    if mode == Stale::Warn {
        for l in &stale {
            eprintln!("warning: {}", sanitize(l));
        }
        return Ok(());
    }
    Err(fail(
        format!(
            "{} of {} corrections match nothing",
            stale.len(),
            rows.len()
        ),
        stale.join("; "),
        "delete the entry, or fix its ends; `stale: warn` in the file lets this pass",
    ))
}
