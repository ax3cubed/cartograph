//! Interface facts (XS-03): routes and HTTP calls the built-in query adapters find in a blob. They
//! are stored per blob in the `fact` table under the extractor `adapters`, whose version names every
//! adapter and its version, so the facts hash and the truncation of a rewritten history treat them
//! like any other fact. The HTTP resolver (`crate::http`) reads them back.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cartograph_facts::FactCache;
use cartograph_facts::adapters::{AdapterSet, Limits, MAX_GENERATED_BYTES};
use cartograph_facts::schema::BlobFact;
use cartograph_git::{GitError, ObjectId, Repo, read_blob};

/// The extractor name in the `fact` table.
pub const EXTRACTOR: &str = "adapters";

/// One blob's interface facts per adapter: (name, version, facts). Only blobs with any are kept.
pub type IfaceFacts = Vec<(String, String, Vec<BlobFact>)>;
pub type Ifaces = HashMap<ObjectId, Arc<IfaceFacts>>;

/// `express@1.0.0+fetch@1.1.0`: changes whenever a built-in adapter does.
pub fn version() -> String {
    AdapterSet::builtin()
        .adapters()
        .iter()
        .map(|a| format!("{}@{}", a.name, a.version))
        .collect::<Vec<_>>()
        .join("+")
}

/// Whether the built-in adapters read a file of this path.
pub fn applies(path: &str) -> bool {
    cartograph_parse::extractor_for(path).is_some()
        && matches!(
            path.rsplit_once('.').map(|x| x.1),
            Some("ts" | "mts" | "cts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "go")
        )
}

/// Interface facts for each (blob, path) not in `seen`, from the fact cache or by running the
/// built-in adapters on the blob.
pub fn extract(
    repo: &Repo,
    items: impl IntoIterator<Item = (ObjectId, String)>,
    cache: &mut FactCache,
    into: &mut Ifaces,
    seen: &mut HashSet<ObjectId>,
) -> Result<(), GitError> {
    let set = AdapterSet::builtin();
    let local = repo.local();
    for (oid, path) in items {
        if !applies(&path) || !seen.insert(oid) {
            continue;
        }
        let got = set.extract(
            cache,
            oid.as_bytes(),
            &path,
            || read_blob(&local, oid, MAX_GENERATED_BYTES as u64).ok(),
            &Limits::default(),
        );
        let mine: IfaceFacts = got
            .facts
            .into_iter()
            .filter(|a| !a.facts.is_empty())
            .map(|a| (a.name, a.version, a.facts))
            .collect();
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
) -> cartograph_store::rusqlite::Result<()> {
    if facts.is_empty() {
        return Ok(());
    }
    let version = version();
    conn.execute(
        "INSERT INTO extractor (name, version) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        cartograph_store::rusqlite::params![EXTRACTOR, version],
    )?;
    let ex: i64 = conn.query_row(
        "SELECT id FROM extractor WHERE name = ?1 AND version = ?2",
        cartograph_store::rusqlite::params![EXTRACTOR, version],
        |r| r.get(0),
    )?;
    let mut rows: Vec<_> = facts.iter().collect();
    rows.sort_by_key(|(o, _)| **o);
    for (oid, f) in rows {
        conn.prepare_cached(
            "INSERT INTO fact (blob, extractor, payload) VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
        )?
        .execute(cartograph_store::rusqlite::params![
            oid.as_bytes(),
            ex,
            serde_json::to_vec(&**f).expect("facts encode")
        ])?;
    }
    Ok(())
}

pub(crate) fn decode(payload: &[u8]) -> Option<IfaceFacts> {
    serde_json::from_slice(payload).ok()
}

/// Whether the adapters that made the stored facts are the ones this binary has.
pub(crate) fn current(
    conn: &cartograph_store::rusqlite::Connection,
) -> cartograph_store::rusqlite::Result<bool> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'adapters_version'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(stored.as_deref() == Some(version().as_str()))
}

pub(crate) fn mark_current(
    conn: &cartograph_store::rusqlite::Connection,
) -> cartograph_store::rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('adapters_version', ?1)",
        [version()],
    )?;
    Ok(())
}
