//! What the index can and can't see (IDX-22), how it was built (IDX-21), and the services it found
//! (IDX-20): the reads behind the Coverage view and the Indexing and Services settings.
//!
//! The import and file rows come from the last finished run's totals; the cross-service rows
//! (calls, handlers, consumers) belong to XS-14 and aren't answered here.

use cartograph_api::types::{CoverageRow, ExtractorInfo, IndexRun, Problem, Service, Unresolved};
use cartograph_core::{NodeId, RepoId};
use cartograph_store::rusqlite::{self, OptionalExtension};

use crate::page::{Page, Paged};
use crate::resolve::node_id;
use crate::{At, QueryError, Snapshot, rfc3339};

/// The coverage row whose gaps are listed by [`Snapshot::unresolved`].
pub const IMPORTS_ROW: &str = "imports";

fn no_row() -> QueryError {
    QueryError::NotFound {
        what: "no such coverage row".into(),
        action: "pass a row id from …/coverage that has a gap".into(),
    }
}

impl Snapshot<'_> {
    /// `GET …/coverage`: files parsed and imports resolved at the last run that recorded totals.
    /// An index with no such run answers no rows.
    pub fn coverage(&self) -> Result<Vec<CoverageRow>, QueryError> {
        let totals: Option<(i64, i64, i64, i64)> = self
            .conn()
            .prepare_cached(
                "SELECT files_total, files_parsed, imports_total, imports_resolved FROM run
                 WHERE files_total IS NOT NULL AND imports_total IS NOT NULL
                 ORDER BY id DESC LIMIT 1",
            )?
            .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()?;
        let Some((ft, fp, it, ir)) = totals else {
            return Ok(Vec::new());
        };
        let n = |v: i64| v.max(0) as u32;
        let open = n(it) - n(ir).min(n(it));
        let gap = (open > 0).then(|| Problem {
            what: format!(
                "{open} unresolved {}",
                if open == 1 { "import" } else { "imports" }
            ),
            why: "no file, package, dependency or built-in matches the specifier".into(),
            action: "check path aliases and workspace packages, then index again".into(),
        });
        Ok(vec![
            CoverageRow {
                id: "files".into(),
                label: "Files parsed".into(),
                done: n(fp),
                total: n(ft),
                target: None,
                gap: None,
            },
            CoverageRow {
                id: IMPORTS_ROW.into(),
                label: "Imports resolved".into(),
                done: n(ir),
                total: n(it),
                target: None,
                gap,
            },
        ])
    }

    /// `GET …/coverage/{row}/unresolved`: the imports that don't resolve at `at`, by path and
    /// line (the unresolved rows are intervals, so a past date lists what was open then).
    pub fn unresolved(
        &self,
        row: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<Unresolved>, QueryError> {
        if row != IMPORTS_ROW {
            return Err(no_row());
        }
        let seq = self.resolve_at(at)?.seq as i64;
        let mut st = self.conn().prepare_cached(
            "SELECT (SELECT id FROM node WHERE node.file = u.file AND node.kind = 'file'),
                    coalesce((SELECT path FROM file_path p WHERE p.file = u.file
                              ORDER BY p.to_seq IS NULL DESC, p.from_seq DESC LIMIT 1), ''),
                    u.line, u.spec, u.reason
             FROM unresolved u WHERE u.from_seq <= ?3 AND (u.to_seq IS NULL OR u.to_seq > ?3)
             ORDER BY 2, u.line, u.spec LIMIT ?1 OFFSET ?2",
        )?;
        let rows = st
            .query_map(
                rusqlite::params![page.limit as i64 + 1, page.offset as i64, seq],
                |r| {
                    Ok(Unresolved {
                        file: NodeId(r.get::<_, Option<i64>>(0)?.map(node_id).unwrap_or_default()),
                        path: r.get(1)?,
                        line: r.get::<_, i64>(2)?.max(0) as u32,
                        text: r.get(3)?,
                        reason: r.get(4)?,
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
            "SELECT n.id, n.kind, n.key, n.name, n.detected_by FROM node n
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
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, key, name, detected_by) = row?;
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
            });
        }
        Ok(out)
    }
}
