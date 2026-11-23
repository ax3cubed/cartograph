//! `cartograph verify` (IDX-17, B4): compares fact hashes. With another index, the two are equal
//! when they hold the same facts, whatever machine, page size or insert order built them; on a
//! mismatch it names the first section that differs. With no argument it recomputes this index's
//! hash and compares it with the one stored at the last sync. The hash proves equal facts, not
//! where they came from.

use std::path::Path;

use cartograph_core::{ExitCode, Report, paths};
use cartograph_discovery::setup;
use cartograph_store::Index;
use cartograph_store::hash::{Verdict, fact_hash, verify};
use cartograph_store::rusqlite::OptionalExtension;
use serde_json::json;

use crate::output::Out;
use crate::{data_dir, default_index, lossy};

pub fn run(out: &Out, dir: &Path, other: Option<&Path>) -> Result<(), Report> {
    let data = data_dir()?;
    let ours = match setup::locate(&data, dir).map_err(|e| Report::from_error(&e))? {
        Some(ws) => paths::workspace_dir(&data, &ws.workspace_id).join("index.cartograph"),
        None => default_index(&data, dir),
    };
    let err = |e: cartograph_store::StoreError| Report::from_error(&e);
    let v = match other {
        Some(theirs) => verify(&ours, theirs).map_err(err)?,
        None => {
            let ix = Index::open_read(&ours).map_err(err)?;
            let sql = |e: cartograph_store::rusqlite::Error| Report {
                what: format!("can't read {}", ours.display()),
                why: format!("{e}"),
                action: "cartograph index --rebuild".into(),
                exit_code: ExitCode::Error,
            };
            // One read transaction: the state, the stored hash and every fact come from the same
            // snapshot, so a sync committing batches meanwhile can't make them disagree.
            ix.conn.execute_batch("BEGIN").map_err(sql)?;
            let meta = |k: &str| -> Result<Option<String>, Report> {
                ix.conn
                    .query_row(
                        "SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1",
                        [k],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(sql)
            };
            // The hash is stored when a sync completes; mid-sync it describes fewer facts.
            if meta("state")?.as_deref() != Some("complete") {
                return Err(Report {
                    what: format!("{} is part way through a sync", ours.display()),
                    why: "its stored hash is written when a sync completes".into(),
                    action: "run `cartograph verify` again once `cartograph status` says current"
                        .into(),
                    exit_code: ExitCode::Error,
                });
            }
            let stored = meta("fact_hash")?;
            let now = fact_hash(&ix.conn).map_err(sql)?.to_string();
            let _ = ix.conn.execute_batch("COMMIT");
            let stored = stored.unwrap_or_else(|| "none".into());
            Verdict {
                first_difference: (stored != now)
                    .then(|| "the hash stored at the last sync".into()),
                ours: now,
                theirs: stored,
            }
        }
    };
    let against = other.map_or_else(|| "its stored hash".into(), |p| p.display().to_string());
    match &v.first_difference {
        None => {
            out.print(
                &format!("same facts: {}", v.ours),
                json!({ "equal": true, "ours": v.ours, "theirs": v.theirs, "index": lossy(&ours) }),
            );
            Ok(())
        }
        Some(section) => Err(Report {
            what: format!("{} and {against} hold different facts", ours.display()),
            why: format!(
                "the first difference is in {section} ({} against {})",
                v.ours, v.theirs
            ),
            action: "run `cartograph sync` on both, at the same commits, and compare again".into(),
            exit_code: ExitCode::Error,
        }),
    }
}
