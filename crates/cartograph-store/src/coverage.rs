//! The six coverage rows (XS-14): files, imports, calls, handlers, routes, consumers.
//!
//! A row's gaps are the `unresolved` rows of its `fact_kind` that are open at a step, counted once
//! per (file, line, spec). Its total is the run's (files, imports) or the step function the HTTP
//! pass writes to `coverage_total` (the rest), so any date's figure comes from intervals; the last
//! run's figures are kept in `coverage_row`. Done is always total minus gaps.

use rusqlite::{Connection, OptionalExtension, params};

/// The rows, in view order, with the `fact_kind` of their gaps (`None`: the imports of IDX-22).
pub const ROWS: [(&str, Option<&str>); 6] = [
    ("files", Some("file")),
    ("imports", None),
    ("calls", Some("http.call")),
    ("handlers", Some("operation.handler")),
    ("routes", Some("http.route")),
    ("consumers", Some("operation.consumer")),
];

/// The target of each row when the manifest sets none, in percent (`coverage.targets`).
pub const DEFAULT_TARGETS: [(&str, Option<f32>); 6] = [
    ("files", Some(98.0)),
    ("imports", Some(99.0)),
    ("calls", Some(85.0)),
    ("handlers", None),
    ("routes", Some(80.0)),
    ("consumers", Some(75.0)),
];

/// One row's figure: `done + unresolved = total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Figure {
    pub done: i64,
    pub total: i64,
    pub unresolved: i64,
}

impl Figure {
    fn new(total: i64, unresolved: i64) -> Self {
        let total = total.max(unresolved);
        Self {
            done: total - unresolved,
            total,
            unresolved,
        }
    }
}

/// What a gap query counts: the rows of `kind` open at `seq`.
const LIVE: &str = "fact_kind IS ?1 AND from_seq <= ?2 AND (to_seq IS NULL OR to_seq > ?2)";

/// The references of `kind` unresolved at `seq`.
pub fn gap_count(conn: &Connection, kind: Option<&str>, seq: i64) -> rusqlite::Result<i64> {
    conn.prepare_cached(&format!(
        "SELECT count(*) FROM (SELECT DISTINCT file, line, spec FROM unresolved WHERE {LIVE})"
    ))?
    .query_row(params![kind, seq], |r| r.get(0))
}

/// The reasons behind the gaps of `kind` at `seq`, most rows first.
pub fn gap_reasons(
    conn: &Connection,
    kind: Option<&str>,
    seq: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(String, i64)>> {
    conn.prepare_cached(&format!(
        "SELECT reason, count(*) AS n FROM
           (SELECT DISTINCT file, line, spec, reason FROM unresolved WHERE {LIVE})
         GROUP BY reason ORDER BY n DESC, reason LIMIT ?3"
    ))?
    .query_map(params![kind, seq, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

/// The six figures at `seq`. `run` is the last run's totals (files total, files parsed, imports
/// total, imports resolved) when `seq` is the run's own; for any other date the files are the
/// paths alive and the imports the import links alive plus the gaps.
pub fn figures(
    conn: &Connection,
    seq: i64,
    run: Option<[i64; 4]>,
) -> rusqlite::Result<[Figure; 6]> {
    let mut out = [Figure::default(); 6];
    for (i, (row, kind)) in ROWS.iter().enumerate() {
        let gaps = gap_count(conn, *kind, seq)?;
        let total = match (*row, run) {
            ("files", Some(r)) => r[0],
            ("imports", Some(r)) => r[2],
            ("files", None) => conn.query_row(
                "SELECT count(*) FROM file_path WHERE from_seq <= ?1 AND (to_seq IS NULL OR to_seq > ?1)",
                [seq],
                |r| r.get(0),
            )?,
            ("imports", None) => {
                let links: i64 = conn.query_row(
                    "SELECT count(*) FROM interval i JOIN link l ON l.id = i.link
                     WHERE l.kind IN ('import', 'package')
                       AND i.from_seq <= ?1 AND (i.to_seq IS NULL OR i.to_seq > ?1)",
                    [seq],
                    |r| r.get(0),
                )?;
                links + gaps
            }
            _ => conn
                .prepare_cached(
                    "SELECT total FROM coverage_total WHERE row = ?1 AND seq <= ?2
                     ORDER BY seq DESC LIMIT 1",
                )?
                .query_row(params![row, seq], |r| r.get(0))
                .optional()?
                .unwrap_or(0),
        };
        out[i] = Figure::new(total, gaps);
    }
    Ok(out)
}

/// `meta` key holding the manifest's `coverage.targets`, one `row=percent` per line.
pub const TARGETS_KEY: &str = "coverage_targets";

/// The target of each row: the manifest's, else the default (`None`: no target). A value outside
/// 0 to 100 is ignored.
pub fn targets(conn: &Connection) -> rusqlite::Result<Vec<(&'static str, Option<f32>)>> {
    let set: String = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1",
            [TARGETS_KEY],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let set: Vec<(&str, f32)> = set
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((
                k,
                v.parse::<f32>()
                    .ok()
                    .filter(|v| (0.0..=100.0).contains(v))?,
            ))
        })
        .collect();
    Ok(DEFAULT_TARGETS
        .iter()
        .map(|(row, d)| {
            let own = set.iter().find(|(k, _)| k == row).map(|(_, v)| *v);
            (*row, own.or(*d))
        })
        .collect())
}

/// `done` as a percentage of `total`; `None` when there is nothing to cover.
pub fn percent(f: &Figure) -> Option<f32> {
    (f.total > 0).then(|| f.done as f32 * 100.0 / f.total as f32)
}
