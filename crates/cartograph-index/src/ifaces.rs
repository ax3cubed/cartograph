//! Interface facts (XS-03): routes and HTTP calls the built-in query adapters find in a blob. They
//! are stored per blob in the `fact` table under the extractor `adapters`, whose version names every
//! adapter and its version, so the facts hash and the truncation of a rewritten history treat them
//! like any other fact. The HTTP resolver (`crate::http`) reads them back.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cartograph_facts::FactCache;
use cartograph_facts::adapters::subprocess::{Session, SubSet};
use cartograph_facts::adapters::{AdapterSet, Limits, MAX_GENERATED_BYTES};
use cartograph_facts::schema::{BlobFact, FactKind};
use cartograph_git::{GitError, ObjectId, Repo, read_blob};

use crate::links::{entries, tables};

/// The `meta` key counting the blobs with a generated-client marker (XS-11). Absent: unknown.
/// It can overcount after a rewrite drops facts (a wasted look), never undercount.
pub(crate) const CODEGEN_BLOBS: &str = "codegen_blobs";

/// The `meta` key counting the blobs with an event fact (XS-16), kept like [`CODEGEN_BLOBS`].
pub(crate) const EVENT_BLOBS: &str = "event_blobs";

/// The extractor name in the `fact` table.
pub const EXTRACTOR: &str = "adapters";

/// One blob's interface facts per adapter: (name, version, facts). Only blobs with any are kept.
pub type IfaceFacts = Vec<(String, String, Vec<BlobFact>)>;
pub type Ifaces = HashMap<ObjectId, Arc<IfaceFacts>>;

/// `express@1.0.0+fetch@1.1.0`: changes whenever a built-in adapter does.
pub fn version(subs: &SubSet) -> String {
    let builtin = AdapterSet::builtin()
        .adapters()
        .iter()
        .map(|a| format!("{}@{}", a.name, a.version))
        .collect::<Vec<_>>()
        .join("+");
    // Approving or removing a subprocess adapter (XS-24) makes the facts again.
    format!("{builtin}{}", subs.version())
}

/// Whether the built-in adapters read a file of this path.
pub fn applies(path: &str) -> bool {
    cartograph_parse::extractor_for(path).is_some()
        && matches!(
            path.rsplit_once('.').map(|x| x.1),
            Some(
                "ts" | "mts"
                    | "cts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "mjs"
                    | "cjs"
                    | "go"
                    | "py"
                    | "php"
                    | "yaml"
                    | "yml"
            )
        )
}

/// The extractor name of the table facts (XS-17), which share [`IfaceFacts`] until [`write`] splits
/// them off: the table pass reads them without the HTTP pass's payloads.
pub const TABLES: &str = "tables";

/// The pseudo-adapter name of the entrypoint facts no query adapter makes (XS-20): they are stored
/// in the `adapters` row of the blob, like any adapter's.
pub const ENTRIES: &str = "entries";

/// Interface facts for each (blob, path) not in `seen`, from the fact cache or by running the
/// built-in adapters, and the approved subprocess adapters (`sub`), on the blob. The table facts
/// (XS-17) of the same blob are an entry named [`TABLES`].
pub fn extract(
    repo: &Repo,
    items: impl IntoIterator<Item = (ObjectId, String)>,
    cache: &mut FactCache,
    into: &mut Ifaces,
    seen: &mut HashSet<ObjectId>,
    sub: &mut Session,
) -> Result<(), GitError> {
    let set = AdapterSet::builtin();
    let local = repo.local();
    for (oid, path) in items {
        let builtin = applies(&path);
        let tables = tables::applies(&path);
        let entry = entries::applies(&path);
        if (!builtin && !tables && !entry && sub.is_idle()) || !seen.insert(oid) {
            continue;
        }
        // One read of the blob for the adapters and the table scan.
        let bytes = std::cell::OnceCell::new();
        let read = || bytes.get_or_init(|| read_blob(&local, oid, MAX_GENERATED_BYTES as u64).ok());
        let mut facts = if builtin {
            set.extract(
                cache,
                oid.as_bytes(),
                &path,
                || read().clone(),
                &Limits::default(),
            )
            .facts
        } else {
            Vec::new()
        };
        facts.extend(sub.extract(cache, oid.as_bytes(), &path, || read().clone()));
        let mut mine: IfaceFacts = facts
            .into_iter()
            .filter(|a| !a.facts.is_empty())
            .map(|a| (a.name, a.version, a.facts))
            .collect();
        if tables
            && let Some(b) = read()
                .as_deref()
                .filter(|b| b.len() as u64 <= cartograph_parse::MAX_BLOB_BYTES)
        {
            let facts = tables::scan(&path, b);
            if !facts.is_empty() {
                mine.push((TABLES.into(), tables::VERSION.into(), facts));
            }
        }
        if entry
            && let Some(b) = read()
                .as_deref()
                .filter(|b| b.len() as u64 <= cartograph_parse::MAX_BLOB_BYTES)
        {
            let facts = entries::scan(&path, b);
            if !facts.is_empty() {
                mine.push((ENTRIES.into(), entries::VERSION.into(), facts));
            }
        }
        if !mine.is_empty() {
            into.insert(oid, Arc::new(mine));
        }
    }
    Ok(())
}

/// Writes the facts, once per blob.
pub(crate) fn write(
    conn: &cartograph_store::rusqlite::Connection,
    facts: &Ifaces,
    subs: &SubSet,
) -> cartograph_store::rusqlite::Result<()> {
    if facts.is_empty() {
        return Ok(());
    }
    let version = version(subs);
    conn.execute(
        "INSERT INTO extractor (name, version) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        cartograph_store::rusqlite::params![EXTRACTOR, version],
    )?;
    let ex: i64 = conn.query_row(
        "SELECT id FROM extractor WHERE name = ?1 AND version = ?2",
        cartograph_store::rusqlite::params![EXTRACTOR, version],
        |r| r.get(0),
    )?;
    let marked = facts
        .values()
        .filter(|f| {
            f.iter()
                .any(|(_, _, fs)| fs.iter().any(|b| b.kind == FactKind::CodegenMarker))
        })
        .count();
    if marked > 0 {
        conn.execute(
            "UPDATE meta SET value = CAST(value AS INTEGER) + ?1 WHERE key = ?2",
            cartograph_store::rusqlite::params![marked as i64, CODEGEN_BLOBS],
        )?;
    }
    let events = facts
        .values()
        .filter(|f| {
            f.iter().any(|(_, _, fs)| {
                fs.iter()
                    .any(|b| matches!(b.kind, FactKind::EventPublish | FactKind::EventSubscribe))
            })
        })
        .count();
    if events > 0 {
        conn.execute(
            "UPDATE meta SET value = CAST(value AS INTEGER) + ?1 WHERE key = ?2",
            cartograph_store::rusqlite::params![events as i64, EVENT_BLOBS],
        )?;
    }
    let mut rows: Vec<_> = facts.iter().collect();
    rows.sort_by_key(|(o, _)| **o);
    // Made with the first table fact, so an index without any has no such extractor.
    let mut tables_id: Option<i64> = None;
    for (oid, f) in rows {
        // The table facts (XS-17) are their own row: one list, read by the table pass alone.
        let (theirs, mine): (IfaceFacts, IfaceFacts) =
            f.iter().cloned().partition(|(name, _, _)| name == TABLES);
        if tables_id.is_none() && !theirs.is_empty() {
            conn.execute(
                "INSERT INTO extractor (name, version) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                cartograph_store::rusqlite::params![TABLES, tables::VERSION],
            )?;
            tables_id = Some(conn.query_row(
                "SELECT id FROM extractor WHERE name = ?1 AND version = ?2",
                cartograph_store::rusqlite::params![TABLES, tables::VERSION],
                |r| r.get(0),
            )?);
        }
        let tables_ex = tables_id.unwrap_or_default();
        let payloads = [
            (ex, (!mine.is_empty()).then(|| serde_json::to_vec(&mine))),
            (tables_ex, theirs.first().map(|t| serde_json::to_vec(&t.2))),
        ];
        for (extractor, payload) in payloads {
            let Some(payload) = payload else { continue };
            conn.prepare_cached(
                "INSERT INTO fact (blob, extractor, payload) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO NOTHING",
            )?
            .execute(cartograph_store::rusqlite::params![
                oid.as_bytes(),
                extractor,
                payload.expect("facts encode")
            ])?;
        }
    }
    Ok(())
}

pub(crate) fn decode(payload: &[u8]) -> Option<IfaceFacts> {
    serde_json::from_slice(payload).ok()
}

/// Whether the adapters that made the stored facts are the ones this binary has.
pub(crate) fn current(
    conn: &cartograph_store::rusqlite::Connection,
    subs: &SubSet,
) -> cartograph_store::rusqlite::Result<bool> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'adapters_version'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(stored.as_deref() == Some(stamp(subs).as_str()))
}

/// What `adapters_version` holds: the adapters' version, the table scan's and the entrypoint scan's.
fn stamp(subs: &SubSet) -> String {
    format!("{}|{}|{}", version(subs), tables::VERSION, entries::VERSION)
}

pub(crate) fn mark_current(
    conn: &cartograph_store::rusqlite::Connection,
    subs: &SubSet,
) -> cartograph_store::rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('adapters_version', ?1)",
        [stamp(subs)],
    )?;
    Ok(())
}
