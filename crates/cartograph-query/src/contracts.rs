//! The Contracts view (XS-08): each spec's operations beside the route that serves them and the
//! calls that consume them, and the routes no spec lists. Read from the `contract`, `operation`
//! and `binding` rows as of a step; the counts are what the view's chips show.
//!
//! Summaries come from the spec: untrusted text (F8), already cut to 200 characters.

use std::collections::BTreeMap;

use cartograph_api::types::{
    Caller, Confidence, Contract, ContractCounts, Method, Operation, OperationStatus, SpecKind,
    UndocumentedRoute,
};
use cartograph_core::NodeId;
use cartograph_store::rusqlite;

use crate::detail::level;
use crate::resolve::node_id;
use crate::{At, QueryError, Snapshot};

fn method(m: &str) -> Method {
    match m.to_ascii_lowercase().as_str() {
        "post" => Method::Post,
        "put" => Method::Put,
        "patch" => Method::Patch,
        "delete" => Method::Delete,
        "options" => Method::Options,
        "head" => Method::Head,
        "trace" => Method::Trace,
        "rpc" => Method::Rpc,
        _ => Method::Get,
    }
}

impl Snapshot<'_> {
    /// `GET …/contracts`: the specs that have operations at `at`, with their operations, the
    /// undocumented routes of their services and the four counts.
    pub fn contracts(&self, at: &At) -> Result<Vec<Contract>, QueryError> {
        let seq = self.resolve_at(at)?.seq as i64;
        let conn = self.conn();
        // `?1` is the step.
        let live =
            |a: &str| format!("{a}.from_seq <= ?1 AND ({a}.to_seq IS NULL OR {a}.to_seq > ?1)");
        type Raw = (i64, String, String, i64, Option<i64>, Option<i64>);
        let specs: Vec<Raw> = conn
            .prepare_cached(&format!(
                "SELECT c.id, c.title,
                        coalesce((SELECT path FROM file_path p WHERE p.file = c.file AND {}
                                  ORDER BY p.from_seq DESC LIMIT 1), ''),
                        (SELECT id FROM node WHERE node.file = c.file AND node.kind = 'file'),
                        c.service, c.file
                 FROM contract c
                 WHERE EXISTS (SELECT 1 FROM operation o WHERE o.contract = c.id AND {})
                 ORDER BY c.id",
                live("p"),
                live("o")
            ))
            .and_then(|mut st| {
                st.query_map([seq], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })?
                .collect()
            })?;

        // What each operation is bound to now: the route's file, the callers' files.
        let mut handler: BTreeMap<i64, i64> = BTreeMap::new();
        let mut callers: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        let mut st = conn.prepare_cached(&format!(
            "SELECT b.operation, b.kind, n.id FROM binding b
             JOIN node n ON n.file = b.file AND n.kind = 'file'
             WHERE b.operation IS NOT NULL AND {} ORDER BY b.id",
            live("b")
        ))?;
        let rows = st.query_map([seq], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (op, kind, node) = r?;
            if kind == "handled_by" {
                handler.entry(op).or_insert(node);
            } else if !callers.entry(op).or_default().contains(&node) {
                callers.get_mut(&op).expect("just made").push(node);
            }
        }
        // The score of the link from a caller to the handler, when there is one.
        let mut link = conn.prepare_cached(
            "SELECT l.score FROM link l JOIN interval i ON i.link = l.id
             WHERE l.kind = 'http' AND l.src = ?1 AND l.dst = ?2
               AND i.from_seq <= ?3 AND (i.to_seq IS NULL OR i.to_seq > ?3) LIMIT 1",
        )?;

        // Routes in a service that has a spec and in none, by service.
        let mut undocumented: BTreeMap<i64, Vec<UndocumentedRoute>> = BTreeMap::new();
        let mut st = conn.prepare_cached(&format!(
            "SELECT coalesce(n.service, 0), b.template, n.id FROM binding b
             JOIN node n ON n.file = b.file AND n.kind = 'file'
             WHERE b.kind = 'undocumented' AND {} ORDER BY b.template, n.id",
            live("b")
        ))?;
        let rows = st.query_map([seq], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (service, template, node) = r?;
            let (m, path) = template.split_once(' ').unwrap_or(("get", &template));
            undocumented
                .entry(service)
                .or_default()
                .push(UndocumentedRoute {
                    method: method(m),
                    path: path.to_owned(),
                    handler: NodeId(node_id(node)),
                });
        }

        let mut out = Vec::new();
        for (id, title, path, file_node, service, _file) in specs {
            let service_node = service.or(Some(file_node)).unwrap_or(file_node);
            let mut ops = Vec::new();
            let mut st = conn.prepare_cached(&format!(
                "SELECT id, method, path, operation_id, summary FROM operation
                 WHERE contract = ?2 AND {} ORDER BY path, method, id",
                live("operation")
            ))?;
            let rows = st.query_map([seq, id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            for r in rows {
                let (op, m, p, operation_id, summary) = r?;
                let h = handler.get(&op).copied();
                let callers = callers
                    .get(&op)
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        let score: Option<f64> = h.and_then(|h| {
                            link.query_row(rusqlite::params![c, h, seq], |r| r.get(0))
                                .ok()
                        });
                        // A call that reaches only the spec has no link to read a score from:
                        // the spec names its target, which is as good as a template match.
                        let score = score.unwrap_or(0.6) as f32;
                        Caller {
                            node: NodeId(node_id(*c)),
                            confidence: Confidence {
                                level: level(score),
                                score,
                                evidence: Vec::new(),
                            },
                        }
                    })
                    .collect();
                ops.push(Operation {
                    id: NodeId(format!("o{op}")),
                    method: method(&m),
                    path: p,
                    operation_id,
                    summary,
                    status: if h.is_some() {
                        OperationStatus::Sync
                    } else {
                        OperationStatus::SpecOnly
                    },
                    handler: h.map(|h| NodeId(node_id(h))),
                    callers,
                });
            }
            // The service's undocumented routes belong to its first spec.
            let routes = service
                .and_then(|s| undocumented.remove(&s))
                .unwrap_or_default();
            let sync = ops
                .iter()
                .filter(|o| o.status == OperationStatus::Sync)
                .count() as u32;
            let counts = ContractCounts {
                operations: ops.len() as u32,
                sync,
                spec_only: ops.len() as u32 - sync,
                undocumented: routes.len() as u32,
            };
            let kind = if path.ends_with(".proto") {
                SpecKind::Proto
            } else {
                SpecKind::Openapi
            };
            out.push(Contract {
                id: format!("c{id}"),
                name: if title.is_empty() {
                    path.rsplit('/').next().unwrap_or(&path).to_owned()
                } else {
                    title
                },
                file: path,
                kind,
                service: NodeId(node_id(service_node)),
                operations: ops,
                undocumented: routes,
                counts,
            });
        }
        Ok(out)
    }
}
