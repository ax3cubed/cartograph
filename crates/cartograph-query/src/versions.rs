//! Spec versions (XS-19): the `contract_version` rows of a spec, each breaking change with the
//! callers of its operation in the commit before; and a full diff of two versions, recomputed from
//! the stored facts of their root blobs (the rows keep only the breaking list and counts).

use cartograph_api::types::{
    BreakingChange, Caller, Confidence, ContractDiff, ContractVersion, SpecChange,
};
use cartograph_core::NodeId;
use cartograph_parse::BlobFacts;
use cartograph_parse::openapi::{self, Contract};
use cartograph_parse::openapi_diff::{self, Change};
use cartograph_store::rusqlite::{OptionalExtension, params};

use crate::detail::level;
use crate::resolve::{hex, node_id};
use crate::{QueryError, Snapshot};

fn kind_name(k: openapi_diff::ChangeKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl Snapshot<'_> {
    fn contract_row(&self, id: &str) -> Result<i64, QueryError> {
        let missing = || QueryError::NotFound {
            what: format!("no contract {id}"),
            action: "list contracts and use one of their ids".into(),
        };
        let n: i64 = id
            .strip_prefix('c')
            .and_then(|n| n.parse().ok())
            .ok_or_else(missing)?;
        self.conn()
            .prepare_cached("SELECT id FROM contract WHERE id = ?1")?
            .query_row([n], |r| r.get(0))
            .optional()?
            .ok_or_else(missing)
    }

    /// `GET …/contracts/{id}/versions`.
    pub fn contract_versions(&self, id: &str) -> Result<Vec<ContractVersion>, QueryError> {
        let contract = self.contract_row(id)?;
        type Raw = (i64, i64, Vec<u8>, i64, i64, String, String);
        let rows: Vec<Raw> = self
            .conn()
            .prepare_cached(
                "SELECT n, seq, spec_hash, changes, breaking_count, breaking, summary
                 FROM contract_version WHERE contract = ?1 ORDER BY n",
            )?
            .query_map([contract], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for (n, seq, hash, changes, count, breaking, summary) in rows {
            let list: Vec<Change> = serde_json::from_str(&breaking).unwrap_or_default();
            let mut breaking = Vec::new();
            for c in list {
                let callers = if c.kind == openapi_diff::ChangeKind::OperationRemoved {
                    self.callers_of(contract, &c.subject, seq - 1)?
                } else {
                    Vec::new()
                };
                breaking.push(BreakingChange {
                    kind: kind_name(c.kind),
                    subject: c.subject,
                    summary: c.summary,
                    callers,
                });
            }
            out.push(ContractVersion {
                n: n as u32,
                commit: self.commit(seq as u32)?,
                spec_hash: hex(&hash),
                changes: changes as u32,
                breaking_count: count as u32,
                summary,
                breaking,
            });
        }
        Ok(out)
    }

    /// Who consumed `method path` of a contract at a step.
    fn callers_of(
        &self,
        contract: i64,
        subject: &str,
        seq: i64,
    ) -> Result<Vec<Caller>, QueryError> {
        let (method, path) = subject.split_once(' ').unwrap_or(("get", subject));
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT DISTINCT n.id,
                    (SELECT l.score FROM binding h
                       JOIN node hn ON hn.file = h.file AND hn.kind = 'file'
                       JOIN link l ON l.kind = 'http' AND l.src = n.id AND l.dst = hn.id
                       JOIN interval i ON i.link = l.id
                      WHERE h.kind = 'handled_by' AND h.operation = o.id
                        AND h.from_seq <= ?4 AND (h.to_seq IS NULL OR h.to_seq > ?4)
                        AND i.from_seq <= ?4 AND (i.to_seq IS NULL OR i.to_seq > ?4) LIMIT 1)
             FROM operation o
               JOIN binding b ON b.operation = o.id AND b.kind = 'consumes'
               JOIN node n ON n.file = b.file AND n.kind = 'file'
             WHERE o.contract = ?1 AND o.method = ?2 AND o.path = ?3
               AND o.from_seq <= ?4 AND (o.to_seq IS NULL OR o.to_seq > ?4)
               AND b.from_seq <= ?4 AND (b.to_seq IS NULL OR b.to_seq > ?4)
             ORDER BY n.id",
        )?;
        let rows = st
            .query_map(params![contract, method, path, seq], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<f64>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(node, score)| {
                // As in the Contracts view: a call that reaches only the spec scores as a template match.
                let score = score.unwrap_or(0.6) as f32;
                Caller {
                    node: NodeId(node_id(node)),
                    confidence: Confidence {
                        level: level(score),
                        score,
                        evidence: Vec::new(),
                    },
                }
            })
            .collect())
    }

    /// The spec of version `n`, from the facts of its root blob.
    fn version_spec(&self, contract: i64, n: u32) -> Result<(Contract, bool), QueryError> {
        let blob: Vec<u8> = self
            .conn()
            .prepare_cached("SELECT blob FROM contract_version WHERE contract = ?1 AND n = ?2")?
            .query_row(params![contract, n], |r| r.get(0))
            .optional()?
            .ok_or_else(|| QueryError::NotFound {
                what: format!("no version {n}"),
                action: "list the versions of the contract".into(),
            })?;
        let payload: Vec<u8> = self
            .conn()
            .prepare_cached(
                "SELECT fa.payload FROM fact fa
                 JOIN extractor e ON e.id = fa.extractor AND e.name = ?1 WHERE fa.blob = ?2",
            )?
            .query_row(params![openapi::KIND, blob], |r| r.get(0))
            .optional()?
            .ok_or(QueryError::Internal)?;
        let Some(BlobFacts::Facts(facts)) = BlobFacts::decode(&payload) else {
            return Err(QueryError::Internal);
        };
        let c = openapi::from_facts(&facts).ok_or(QueryError::Internal)?;
        let approximate = !c.files.is_empty();
        Ok((c, approximate))
    }

    /// `GET …/contracts/{id}/diff?from=&to=`: every change, breaking or not.
    pub fn contract_diff(&self, id: &str, from: u32, to: u32) -> Result<ContractDiff, QueryError> {
        let contract = self.contract_row(id)?;
        let (a, x) = self.version_spec(contract, from)?;
        let (b, y) = self.version_spec(contract, to)?;
        Ok(ContractDiff {
            from,
            to,
            approximate: x || y,
            changes: openapi_diff::diff(&a, &b)
                .into_iter()
                .map(|c| SpecChange {
                    kind: kind_name(c.kind),
                    subject: c.subject,
                    breaking: c.breaking,
                    summary: c.summary,
                })
                .collect(),
        })
    }
}
