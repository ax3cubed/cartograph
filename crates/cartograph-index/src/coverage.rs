//! Coverage accounting (XS-14): the skipped files as unresolved rows, the six-row figure at the
//! last run as a derived row, and the sync's one-line summary.
//!
//! The other rows' gaps are written by the passes that find them: imports by the replay's event
//! stream (IDX-22), calls, routes and operations by the HTTP pass.

use std::collections::{BTreeMap, HashMap};

use cartograph_facts::Replay;
use cartograph_parse::{BlobFacts, SkipReason, extractor_for};
use cartograph_store::coverage::{self as cov, Figure, ROWS, TARGETS_KEY, percent};
use cartograph_store::derived::{changed, record, stored};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};

use crate::derive::key;
use crate::rows::cut;

const VERSION: &str = "coverage 1";

/// The files the index counts that were not parsed, and why: the gaps of the `files` row.
pub fn skipped(replay: &Replay) -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    for (path, f) in replay.tree() {
        if let BlobFacts::Skipped(r) = &**f
            && *r != SkipReason::Generated
            && extractor_for(path).is_some_and(|(_, k)| !k.starts_with("manifest-"))
        {
            out.insert(
                path.clone(),
                match r {
                    SkipReason::TooLarge => "too-large",
                    SkipReason::Binary => "binary",
                    SkipReason::LfsPointer => "lfs-pointer",
                    SkipReason::Generated => "generated",
                    SkipReason::Timeout => "timeout",
                },
            );
        }
    }
    out
}

/// Opens a row for each file skipped now and closes the rows of those that no longer are.
pub fn write_skipped(
    conn: &Connection,
    skipped: &BTreeMap<String, &'static str>,
    seq: i64,
) -> rusqlite::Result<()> {
    let open: HashMap<(String, String), i64> = conn
        .prepare(
            "SELECT spec, reason, rowid FROM unresolved WHERE fact_kind = 'file' AND to_seq IS NULL",
        )?
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    for ((spec, reason), id) in &open {
        if skipped.get(spec).is_none_or(|r| r != reason) {
            conn.execute(
                "UPDATE unresolved SET to_seq = ?2 WHERE rowid = ?1",
                params![id, seq],
            )?;
        }
    }
    for (path, reason) in skipped {
        let spec = cut(path);
        if open.contains_key(&(spec.clone(), (*reason).to_owned())) {
            continue;
        }
        let file: Option<i64> = conn
            .prepare_cached("SELECT file FROM file_path WHERE path = ?1 AND to_seq IS NULL")?
            .query_row([path], |r| r.get(0))
            .optional()?;
        if let Some(file) = file {
            conn.prepare_cached(
                "INSERT INTO unresolved (file, spec, line, stage, reason, from_seq, fact_kind, raw)
                 VALUES (?1, ?2, 0, 'parse', ?3, ?4, 'file', ?2)",
            )?
            .execute(params![file, spec, reason, seq])?;
        }
    }
    Ok(())
}

/// Rebuilds `coverage_row` (the six figures at `seq`) when what they count changed: the step, the
/// run's totals, the facts, the HTTP pass's key or the manifest's targets.
pub(crate) fn refresh(
    conn: &Connection,
    seq: i64,
    hash: &[u8],
    targets: &BTreeMap<String, f32>,
) -> rusqlite::Result<bool> {
    let run: Option<[i64; 4]> = conn
        .query_row(
            "SELECT files_total, files_parsed, imports_total, imports_resolved FROM run
             WHERE files_total IS NOT NULL AND imports_total IS NOT NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
        )
        .optional()?;
    let set: String = targets.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let k = key(
        "coverage",
        VERSION,
        vec![
            format!("{seq}\0{run:?}\0{set}").into_bytes(),
            hash.to_vec(),
            stored(conn, "http", "workspace")?.unwrap_or_default(),
        ],
        None,
    );
    if !changed(conn, "coverage", "workspace", &k)? {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![TARGETS_KEY, set],
    )?;
    conn.execute("DELETE FROM coverage_row", [])?;
    for ((row, _), f) in ROWS.iter().zip(cov::figures(conn, seq, run)?) {
        conn.prepare_cached(
            "INSERT INTO coverage_row (row, seq, done, total, unresolved) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![row, seq, f.done, f.total, f.unresolved])?;
    }
    record(conn, "coverage", "workspace", &k)?;
    Ok(true)
}

/// The figure of each row at the last run.
pub fn rows(conn: &Connection) -> rusqlite::Result<Vec<(&'static str, Figure)>> {
    let mut st = conn.prepare("SELECT done, total, unresolved FROM coverage_row WHERE row = ?1")?;
    let mut out = Vec::new();
    for (row, _) in ROWS {
        if let Some(f) = st
            .query_row([row], |r| {
                Ok(Figure {
                    done: r.get(0)?,
                    total: r.get(1)?,
                    unresolved: r.get(2)?,
                })
            })
            .optional()?
        {
            out.push((row, f));
        }
    }
    Ok(out)
}

/// The line `sync` prints (D12): each row's figure, then the rows under their target.
pub fn summary(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let rows = rows(conn)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let targets = cov::targets(conn)?;
    let (mut parts, mut below) = (Vec::new(), Vec::new());
    for (row, f) in &rows {
        let Some(p) = percent(f) else {
            parts.push(format!("{row} n/a"));
            continue;
        };
        parts.push(format!("{row} {p:.1}%"));
        if let Some(t) = targets.iter().find(|(r, _)| r == row).and_then(|(_, t)| *t)
            && p < t
        {
            below.push(format!("{row} {p:.1}% < {t}%"));
        }
    }
    let mut line = format!("coverage: {}", parts.join(", "));
    if !below.is_empty() {
        line += &format!("; under target: {}", below.join(", "));
    }
    Ok(Some(line))
}
