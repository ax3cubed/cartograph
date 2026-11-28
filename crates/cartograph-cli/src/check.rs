//! `cartograph check` (XS-12, XS-22): fails on a correction that matches nothing and on an
//! architecture rule violation that isn't in the baseline. `--update-baseline` rewrites the
//! baseline to the violations there are now.
//!
//! The statuses are the index's, written by the last sync. The file is read here too, so one that
//! doesn't parse fails with its line, and so does an index that is behind the file.

use std::path::Path;

use cartograph_core::{ExitCode, Report};
use cartograph_discovery::overrides::{self, Stale};
use cartograph_discovery::setup;
use cartograph_index::corrections::{self, Row};
use cartograph_index::rules;
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

pub fn run(out: &Out, dir: &Path, update_baseline: bool) -> Result<(), Report> {
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
    let manifest =
        cartograph_server::manifest_path(&data, &applied.workspace_id).ok_or_else(|| {
            fail(
                "the workspace has no manifest".into(),
                String::new(),
                "run `cartograph init`",
            )
        })?;
    let rep = rules::report(&manifest, &ws.index_path()).map_err(|(w, y, a)| fail(w, y, &a))?;
    if update_baseline {
        let b = rules::current_baseline(&rep);
        write_baseline(&rep.baseline_path, &b.to_json())?;
        out.print(
            &format!(
                "wrote {}: {} violations, {} fixed ones dropped",
                rep.baseline_path.display(),
                b.violations.len(),
                rep.fixed.len()
            ),
            json!({ "corrections": corrections_json, "rules": rules_json(&rep), "fixed": [], "baseline_written": b.violations.len() }),
        );
        return Ok(());
    }
    let line_of = |f: &rules::Found| {
        let v = &f.violation;
        format!(
            "{} {}: {} {} -> {}{}",
            v.rule,
            f.severity.as_str(),
            v.kind,
            sanitize(&v.from),
            sanitize(&v.to),
            if v.op.is_empty() {
                String::new()
            } else {
                format!(" ({})", sanitize(&v.op))
            }
        )
    };
    let new: Vec<&rules::Found> = rep.new_ones().collect();
    let (errors, soft): (Vec<_>, Vec<_>) = new
        .iter()
        .copied()
        .partition(|f| f.severity == cartograph_discovery::rules::Severity::Error);
    let mut human = human;
    if !rep.rules.is_empty() {
        human.push_str(&format!(
            "\n{} rules: {} violations, {} baselined, {} new",
            rep.rules.len(),
            rep.rules.iter().map(|r| r.violations).sum::<usize>(),
            rep.rules.iter().map(|r| r.baselined).sum::<usize>(),
            new.len()
        ));
        for f in &soft {
            human.push_str(&format!("\n{}", line_of(f)));
        }
        for e in &rep.fixed {
            human.push_str(&format!(
                "\nfixed: {} {} {} -> {}",
                e.rule,
                e.kind,
                sanitize(&e.from),
                sanitize(&e.to)
            ));
        }
    }
    out.print(
        &human,
        json!({
            "corrections": corrections_json,
            "rules": rules_json(&rep),
            "fixed": rep.fixed.iter().map(|e| json!({ "rule": e.rule, "kind": e.kind, "from": e.from, "to": e.to, "op": e.op })).collect::<Vec<_>>(),
        }),
    );

    // An entry that can't be applied is said once, on stderr; `stale: off` keeps quiet about
    // the stale ones.
    for l in &ambiguous {
        eprintln!("warning: {}", sanitize(l));
    }
    let stale_fails = mode == Stale::Error && !stale.is_empty();
    if mode == Stale::Warn {
        for l in &stale {
            eprintln!("warning: {}", sanitize(l));
        }
    }
    // Only an error-severity violation that the baseline doesn't cover fails.
    if !errors.is_empty() {
        return Err(fail(
            format!("{} architecture rule violations", errors.len()),
            errors
                .iter()
                .map(|f| line_of(f))
                .collect::<Vec<_>>()
                .join("; "),
            "remove the dependency, or accept it with `cartograph check --update-baseline`",
        ));
    }
    if stale_fails {
        return Err(fail(
            format!(
                "{} of {} corrections match nothing",
                stale.len(),
                rows.len()
            ),
            stale.join("; "),
            "delete the entry, or fix its ends; `stale: warn` in the file lets this pass",
        ));
    }
    Ok(())
}

/// What `rules` holds in `--json`: each rule with its live violations.
fn rules_json(rep: &rules::Report) -> Vec<serde_json::Value> {
    rep.rules
        .iter()
        .map(|r| {
            let items: Vec<_> = rep
                .found
                .iter()
                .filter(|f| f.violation.live && f.violation.rule == r.rule.name)
                .map(|f| {
                    let v = &f.violation;
                    json!({
                        "kind": v.kind, "from": v.from, "to": v.to, "op": v.op,
                        "level": v.level, "baselined": f.baselined,
                    })
                })
                .collect();
            json!({
                "name": r.rule.name, "type": r.rule.kind.as_str(),
                "severity": r.rule.severity.as_str(),
                "violations": r.violations, "baselined": r.baselined, "items": items,
                // The reason is repo text: untrusted (F8).
                "untrusted_text": r.rule.reason,
            })
        })
        .collect()
}

/// Writes the baseline through a temp file renamed over the name, never following a link.
fn write_baseline(path: &Path, text: &str) -> Result<(), Report> {
    let io = |e: std::io::Error| {
        fail(
            format!("can't write {}", path.display()),
            e.to_string(),
            "check the folder's permissions",
        )
    };
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(io(std::io::Error::other("it isn't a regular file")));
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}
