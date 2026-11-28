//! What the index can and can't see (IDX-22, XS-14), how it was built (IDX-21), and the services it
//! found (IDX-20): the reads behind the Coverage view and the Indexing and Services settings.
//!
//! Coverage is XS-14's six rows; the other reads are the runs, extractors and services.

use cartograph_api::types::{
    CoverageRow, ExtractorInfo, GapGroup, IndexRun, Problem, Service, Unresolved,
};
use cartograph_core::{NodeId, RepoId};
use cartograph_store::coverage::{self, ROWS};
use cartograph_store::rusqlite;

use crate::page::{Page, Paged};
use crate::resolve::node_id;
use crate::{At, QueryError, Snapshot, rfc3339};

fn no_row() -> QueryError {
    QueryError::NotFound {
        what: "no such coverage row".into(),
        action: "pass a row id from …/coverage".into(),
    }
}

/// What a row is called, and what a gap there is is called and fixed by.
fn words(row: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match row {
        "files" => ("Files parsed", "file", "files", "Show files"),
        "imports" => ("Imports resolved", "import", "imports", "Show imports"),
        "calls" => (
            "Call sites to an operation",
            "call site",
            "call sites",
            "Show calls",
        ),
        "handlers" => (
            "Operations with a handler",
            "operation",
            "operations",
            "Open contract",
        ),
        "routes" => ("Routes in a contract", "route", "routes", "Open contract"),
        _ => (
            "Entrypoints with a consumer",
            "entrypoint",
            "entrypoints",
            "Declare clients",
        ),
    }
}

/// Repo text as shown: masked again and cut to 200 characters (XS-14).
fn shown(s: &str) -> String {
    cartograph_core::secrets::mask(s)
        .chars()
        .take(200)
        .collect()
}

fn n(v: i64) -> u32 {
    v.clamp(0, u32::MAX as i64) as u32
}

impl Snapshot<'_> {
    /// `GET …/coverage`: the six rows at `at`, each with its target and its gaps by reason (XS-14).
    /// The last run's figures are read as they were derived; any other date is counted from the
    /// intervals. An index with no run answers no rows.
    pub fn coverage(&self, at: &At) -> Result<Vec<CoverageRow>, QueryError> {
        let seq = self.resolve_at(at)?.seq as i64;
        let c = self.conn();
        let stored: Option<i64> =
            c.query_row("SELECT min(seq) FROM coverage_row", [], |r| r.get(0))?;
        let figures = if stored == Some(seq) {
            let mut st = c.prepare_cached(
                "SELECT done, total, unresolved FROM coverage_row WHERE row = ?1",
            )?;
            let mut out = [coverage::Figure::default(); 6];
            for (i, (row, _)) in ROWS.iter().enumerate() {
                out[i] = st.query_row([row], |r| {
                    Ok(coverage::Figure {
                        done: r.get(0)?,
                        total: r.get(1)?,
                        unresolved: r.get(2)?,
                    })
                })?;
            }
            out
        } else if stored.is_none() {
            return Ok(Vec::new());
        } else {
            coverage::figures(c, seq, None)?
        };
        let targets = coverage::targets(c)?;
        let mut out = Vec::new();
        for (((row, kind), f), (_, target)) in ROWS.iter().zip(figures).zip(targets) {
            let (label, one, many, action) = words(row);
            let groups = coverage::gap_reasons(c, *kind, seq, 20)?;
            let gap = groups.first().map(|(reason, rows)| Problem {
                what: format!("{rows} {}", if *rows == 1 { one } else { many }),
                why: shown(reason),
                action: action.into(),
            });
            out.push(CoverageRow {
                id: (*row).into(),
                label: label.into(),
                done: n(f.done),
                total: n(f.total),
                target,
                gap,
                gaps: groups
                    .into_iter()
                    .map(|(reason, rows)| GapGroup {
                        reason: shown(&reason),
                        rows: n(rows),
                    })
                    .collect(),
            });
        }
        Ok(out)
    }

    /// `GET …/coverage/{row}/unresolved`: the references behind a row's gap at `at`, by path and
    /// line (the unresolved rows are intervals, so a past date lists what was open then). One per
    /// (file, line, text), so the list is as long as the row's gap count.
    pub fn unresolved(
        &self,
        row: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<Unresolved>, QueryError> {
        let kind = ROWS.iter().find(|(r, _)| *r == row).ok_or_else(no_row)?.1;
        let seq = self.resolve_at(at)?.seq as i64;
        let mut st = self.conn().prepare_cached(
            "SELECT (SELECT id FROM node WHERE node.file = u.file AND node.kind = 'file'),
                    coalesce((SELECT path FROM file_path p WHERE p.file = u.file
                              ORDER BY p.to_seq IS NULL DESC, p.from_seq DESC LIMIT 1), ''),
                    u.line, coalesce(u.raw, u.spec), min(u.reason)
             FROM unresolved u
             WHERE u.fact_kind IS ?4 AND u.from_seq <= ?3 AND (u.to_seq IS NULL OR u.to_seq > ?3)
             GROUP BY u.file, u.line, u.spec
             ORDER BY 2, u.line, u.spec LIMIT ?1 OFFSET ?2",
        )?;
        let rows = st
            .query_map(
                rusqlite::params![page.limit as i64 + 1, page.offset as i64, seq, kind],
                |r| {
                    Ok(Unresolved {
                        file: NodeId(r.get::<_, Option<i64>>(0)?.map(node_id).unwrap_or_default()),
                        path: r.get(1)?,
                        line: r.get::<_, i64>(2)?.max(0) as u32,
                        text: shown(&r.get::<_, String>(3)?),
                        reason: shown(&r.get::<_, String>(4)?),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = rows.len() > page.limit;
        Ok(Paged {
            rows: rows.into_iter().take(page.limit).collect(),
            next_cursor: more.then(|| format!("o{}", page.offset + page.limit)),
        })
    }

    /// `GET …/runs`: index runs, newest first.
    pub fn runs(&self, page: Page) -> Result<Paged<IndexRun>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT id, started_at, finished_at, writer_version,
                    files_total, files_parsed, imports_total, imports_resolved
             FROM run ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        )?;
        let n = |v: Option<i64>| v.map(|v| v.max(0) as u32);
        let rows = st
            .query_map(
                rusqlite::params![page.limit as i64 + 1, page.offset as i64],
                |r| {
                    Ok(IndexRun {
                        id: r.get::<_, i64>(0)?.max(0) as u32,
                        started_at: rfc3339(r.get(1)?),
                        finished_at: r.get::<_, Option<i64>>(2)?.map(rfc3339),
                        writer_version: r.get(3)?,
                        files_total: n(r.get(4)?),
                        files_parsed: n(r.get(5)?),
                        imports_total: n(r.get(6)?),
                        imports_resolved: n(r.get(7)?),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = rows.len() > page.limit;
        Ok(Paged {
            rows: rows.into_iter().take(page.limit).collect(),
            next_cursor: more.then(|| format!("o{}", page.offset + page.limit)),
        })
    }

    /// `GET …/extractors`: each extractor family (`go-imports` for `go-imports/go`), its newest
    /// version, and the live files whose content it read. A family no live file needs is still
    /// listed, with 0.
    pub fn extractors(&self) -> Result<Vec<ExtractorInfo>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "WITH x AS (
               SELECT id, version,
                      CASE WHEN instr(name, '/') > 0 THEN substr(name, 1, instr(name, '/') - 1)
                           ELSE name END AS fam
               FROM extractor)
             SELECT e.fam,
                    (SELECT version FROM x y WHERE y.fam = e.fam ORDER BY y.id DESC LIMIT 1),
                    coalesce(c.files, 0)
             FROM (SELECT DISTINCT fam FROM x) e
             LEFT JOIN (SELECT x.fam, count(DISTINCT v.file) AS files
                        FROM file_version v
                        JOIN fact f ON f.blob = v.blob
                        JOIN x ON x.id = f.extractor
                        WHERE v.to_seq IS NULL GROUP BY x.fam) c ON c.fam = e.fam
             ORDER BY e.fam",
        )?;
        let rows = st.query_map([], |r| {
            Ok(ExtractorInfo {
                name: r.get(0)?,
                version: r.get(1)?,
                files: r.get::<_, i64>(2)?.max(0) as u32,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `GET …/services`: the services and libraries alive now, with the rule that found each.
    pub fn services(&self) -> Result<Vec<Service>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT n.id, n.kind, n.key, n.name, n.detected_by,
                    n.detect_source FROM node n
             WHERE n.kind IN ('service', 'library')
               AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id AND s.to_seq IS NULL)
             ORDER BY n.key",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, key, name, detected_by, detect_source) = row?;
            // `service:<repo>:<root>`, as IDX-20 writes it for services and libraries alike.
            let mut parts = key.as_deref().unwrap_or_default().splitn(3, ':').skip(1);
            let repo = parts.next().unwrap_or_default().to_owned();
            let root = parts.next().unwrap_or_default().to_owned();
            out.push(Service {
                id: NodeId(node_id(id)),
                name: name.unwrap_or_else(|| {
                    if root.is_empty() {
                        repo.clone()
                    } else {
                        root.clone()
                    }
                }),
                repo: RepoId(repo),
                paths: if root.is_empty() { vec![] } else { vec![root] },
                domain: None,
                tags: vec![],
                library: kind == "library",
                detected_by,
                detect_source,
            });
        }
        Ok(out)
    }
}
