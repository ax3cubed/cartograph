//! The Entrypoints view (XS-20): what starts work in a service, who calls it, and what it reaches.
//!
//! The rows are the indexer's (`entrypoint`). Consumers are links into the entrypoint's operation
//! or channel at the date, plus the manifest's declared clients. Reach is a bounded walk
//! (`cartograph_core::reach`, 64 hops, 50,000 nodes) over the links valid at the date, run here on
//! the generation's in-memory graph: one graph covers every date and environment, since each edge
//! carries its interval and environment mask, so nothing is built per date or environment. Only
//! HEAD's count is stored, so a list at `now` walks nothing. Results are ids only.

use std::collections::BTreeMap;

use cartograph_api::types::Reach;
use cartograph_api::types::{
    Consumer, Entrypoint, EntrypointCounts, EntrypointDetail, EntrypointKind, KindCount,
    ReachSummary, ServiceReach,
};
use cartograph_core::reach::{Reached, bfs};
use cartograph_core::{LinkId, NodeId};
use cartograph_store::rusqlite::{OptionalExtension, params};

use crate::adjacency::{Adjacency, NONE};
use crate::detail::level;
use crate::resolve::{link_id, node_id};
use crate::{At, Page, Paged, QueryError, Snapshot};

/// Entrypoints a view lists at most.
const MAX_ENTRYPOINTS: usize = 20_000;

/// What the list is cut to: a kind, a case-insensitive substring of the label, and an order.
#[derive(Debug, Clone, Default)]
pub struct EntryFilter {
    pub kind: Option<EntrypointKind>,
    pub q: String,
    pub sort: EntrySort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EntrySort {
    /// Kind, then label.
    #[default]
    Name,
    Consumers {
        desc: bool,
    },
    Reach {
        desc: bool,
    },
}

impl EntryFilter {
    /// `kind` is one of http, event, job, cli, export, ui. `sort` is name, consumers or reach, and
    /// a leading `-` reverses the last two.
    pub fn parse(
        kind: Option<&str>,
        q: Option<&str>,
        sort: Option<&str>,
    ) -> Result<Self, QueryError> {
        let kind = match kind.filter(|k| !k.is_empty()) {
            None => None,
            Some("http") => Some(EntrypointKind::Http),
            Some("event") => Some(EntrypointKind::Event),
            Some("job") => Some(EntrypointKind::Job),
            Some("cli") => Some(EntrypointKind::Cli),
            Some("export") => Some(EntrypointKind::Export),
            Some("ui") => Some(EntrypointKind::Ui),
            Some(_) => {
                return Err(QueryError::bad(
                    "no such entrypoint kind",
                    "kinds are http, event, job, cli, export and ui",
                    "pass one of them, or none",
                ));
            }
        };
        let sort = match sort.unwrap_or("name") {
            "name" | "" => EntrySort::Name,
            "consumers" => EntrySort::Consumers { desc: false },
            "-consumers" => EntrySort::Consumers { desc: true },
            "reach" => EntrySort::Reach { desc: false },
            "-reach" => EntrySort::Reach { desc: true },
            _ => {
                return Err(QueryError::bad(
                    "no such sort",
                    "sorts are name, consumers, -consumers, reach and -reach",
                    "pass one of them, or none",
                ));
            }
        };
        Ok(Self {
            kind,
            q: q.unwrap_or_default().chars().take(200).collect(),
            sort,
        })
    }
}

/// One entrypoint live at a step.
struct Row {
    uid: String,
    kind: EntrypointKind,
    node: i64,
    service: Option<i64>,
    name: String,
    key: String,
    stored: Option<i64>,
}

fn kind(s: &str) -> EntrypointKind {
    match s {
        "http" => EntrypointKind::Http,
        "event" => EntrypointKind::Event,
        "job" => EntrypointKind::Job,
        "cli" => EntrypointKind::Cli,
        "export" => EntrypointKind::Export,
        _ => EntrypointKind::Ui,
    }
}

const LIVE: &str = "EXISTS (SELECT 1 FROM interval i WHERE i.link = l.id AND i.from_seq <= ?1
                      AND (i.to_seq IS NULL OR i.to_seq > ?1))";

/// What `node` reaches at `seq` over edges that hold in the environments `env` selects (0: all).
fn reached(g: &Adjacency, node: u32, seq: u32, env: u32) -> Reached {
    bfs(g.row.len(), node, |n, push| {
        for e in g.out(n) {
            if e.live(seq) && (env == 0 || e.envs == 0 || e.envs & env != 0) {
                push(e.dst);
            }
        }
    })
}

impl Snapshot<'_> {
    fn entry_rows(&self, seq: i64, uid: Option<&str>) -> Result<Vec<Row>, QueryError> {
        let rows: Vec<Row> = self
            .conn()
            .prepare_cached(
                "SELECT uid, kind, node, service, name, key, reach FROM entrypoint
                 WHERE from_seq <= ?1 AND (to_seq IS NULL OR to_seq > ?1)
                   AND (?2 = '' OR uid = ?2)
                 ORDER BY kind, name, uid LIMIT ?3",
            )?
            .query_map(
                params![seq, uid.unwrap_or(""), MAX_ENTRYPOINTS as i64 + 1],
                |r| {
                    Ok(Row {
                        uid: r.get(0)?,
                        kind: kind(&r.get::<_, String>(1)?),
                        node: r.get(2)?,
                        service: r.get(3)?,
                        name: r.get(4)?,
                        key: r.get(5)?,
                        stored: r.get(6)?,
                    })
                },
            )?
            .collect::<Result<_, _>>()?;
        self.check_rows(rows.len(), "pass a later or earlier at")?;
        Ok(rows)
    }

    /// The links into the entrypoint's operation or channel at `seq` (from the other end's side),
    /// and the clients the manifest declares for it.
    fn consumers(&self, e: &Row, seq: i64) -> Result<Vec<Consumer>, QueryError> {
        let c = self.conn();
        // (link, the calling file, score, layer)
        let map = |r: &cartograph_store::rusqlite::Row<'_>| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        };
        let found: Vec<_> = match e.kind {
            EntrypointKind::Http => c
                .prepare_cached(&format!(
                    "SELECT l.id, l.src, l.score, l.layer FROM link l
                     WHERE l.kind = 'http' AND l.dst = ?2 AND {LIVE}
                       AND (l.layer = 'correction' OR EXISTS (
                         SELECT 1 FROM evidence ev JOIN node n ON n.file = ev.file
                         WHERE ev.link = l.id AND n.id = ?2 AND ev.operation = ?3))
                     ORDER BY l.id"
                ))?
                .query_map(params![seq, e.node, e.key], map)?
                .collect::<Result<_, _>>()?,
            EntrypointKind::Event => {
                let (broker, name) = e.key.split_once('\0').unwrap_or_default();
                c.prepare_cached(&format!(
                    "SELECT l.id, l.dst, l.score, l.layer FROM link l
                     WHERE l.kind = 'event' AND l.src = ?2 AND {LIVE}
                       AND EXISTS (SELECT 1 FROM node n JOIN channel_edge ce ON ce.file = n.file
                         WHERE n.id = l.dst AND ce.role = 'publish' AND ce.broker = ?3
                           AND ce.name = ?4 AND ce.from_seq <= ?1
                           AND (ce.to_seq IS NULL OR ce.to_seq > ?1))
                     ORDER BY l.id"
                ))?
                .query_map(params![seq, e.node, broker, name], map)?
                .collect::<Result<_, _>>()?
            }
            _ => c
                .prepare_cached(&format!(
                    "SELECT l.id, l.src, l.score, l.layer FROM link l
                     WHERE l.dst = ?2 AND l.src != ?2 AND {LIVE} ORDER BY l.id"
                ))?
                .query_map(params![seq, e.node], map)?
                .collect::<Result<_, _>>()?,
        };
        let mut out: Vec<Consumer> = found
            .into_iter()
            .map(|(l, n, score, layer)| Consumer {
                node: Some(NodeId(node_id(n))),
                link: Some(LinkId(link_id(l))),
                name: None,
                level: level(score.unwrap_or(1.0) as f32),
                overridden: layer.as_deref() == Some("correction"),
            })
            .collect();
        if e.kind == EntrypointKind::Http {
            let names: Vec<String> = c
                .prepare_cached("SELECT name FROM entrypoint_client WHERE uid = ?1 ORDER BY name")?
                .query_map([&e.uid], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            out.extend(names.into_iter().map(|name| Consumer {
                node: None,
                link: None,
                name: Some(name),
                level: cartograph_api::types::ConfidenceLevel::Exact,
                overridden: true,
            }));
        }
        Ok(out)
    }

    /// The environment's bit in an edge's mask; 0 (every one) when none is asked for.
    fn env_mask(&self, env: Option<&str>) -> Result<u32, QueryError> {
        let Some(name) = env.filter(|e| !e.is_empty()) else {
            return Ok(0);
        };
        let id: Option<i64> = self
            .conn()
            .prepare_cached("SELECT id FROM environment WHERE name = ?1")?
            .query_row([name], |r| r.get(0))
            .optional()?;
        id.map(|i| 1u32 << (i as u32 % 32)).ok_or_else(|| {
            QueryError::bad(
                "no such environment",
                "env names one of the workspace's environments",
                "pass an env from …/environments",
            )
        })
    }

    fn wire(e: &Row, consumers: u32, reach: u32) -> Entrypoint {
        Entrypoint {
            id: e.uid.clone(),
            kind: e.kind,
            label: e.name.clone(),
            node: NodeId(node_id(e.node)),
            service: e.service.map(|s| NodeId(node_id(s))),
            consumers,
            reach,
        }
    }

    /// One list row: the consumer count, and the reach (HEAD's stored count at `now`, else a walk).
    fn listed(
        &self,
        g: &Adjacency,
        e: &Row,
        seq: u32,
        now: bool,
    ) -> Result<Entrypoint, QueryError> {
        let n = self.consumers(e, i64::from(seq))?.len() as u32;
        let reach = match (now, e.stored, g.index.get(&e.node)) {
            (true, Some(r), _) => r as u32,
            (_, _, Some(&i)) => reached(g, i, seq, 0).nodes.len() as u32,
            _ => 0,
        };
        Ok(Self::wire(e, n, reach))
    }

    /// `GET …/entrypoints`: every entrypoint at `at`, with its consumer count and its reach.
    pub fn entrypoints(&self, at: &At) -> Result<Vec<Entrypoint>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let now = matches!(at, At::Now);
        self.entry_rows(c.seq as i64, None)?
            .iter()
            .map(|e| self.listed(&g, e, c.seq as u32, now))
            .collect()
    }

    /// `GET …/entrypoints?kind=&q=&sort=&limit=&cursor=`: one page. Filtering and the name order
    /// read only the rows; consumers and reach are counted for the page, or for every match when
    /// the sort is by one of them.
    pub fn entrypoints_page(
        &self,
        at: &At,
        f: &EntryFilter,
        page: Page,
    ) -> Result<Paged<Entrypoint>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let (seq, now) = (c.seq as u32, matches!(at, At::Now));
        let needle = f.q.to_lowercase();
        let mut rows = self.entry_rows(c.seq as i64, None)?;
        rows.retain(|e| {
            f.kind.is_none_or(|k| k == e.kind)
                && (needle.is_empty() || e.name.to_lowercase().contains(&needle))
        });
        let desc = match f.sort {
            EntrySort::Name => {
                let p = page.take(rows);
                return Ok(Paged {
                    rows: p
                        .rows
                        .iter()
                        .map(|e| self.listed(&g, e, seq, now))
                        .collect::<Result<_, _>>()?,
                    next_cursor: p.next_cursor,
                });
            }
            EntrySort::Consumers { desc } | EntrySort::Reach { desc } => desc,
        };
        let mut all = rows
            .iter()
            .map(|e| self.listed(&g, e, seq, now))
            .collect::<Result<Vec<_>, _>>()?;
        let key = |e: &Entrypoint| match f.sort {
            EntrySort::Reach { .. } => e.reach,
            _ => e.consumers,
        };
        // A stable sort: ties stay in name order.
        all.sort_by(|a, b| {
            let o = key(a).cmp(&key(b));
            if desc { o.reverse() } else { o }
        });
        Ok(page.take(all))
    }

    /// `GET …/entrypoints/counts`.
    pub fn entrypoint_counts(&self, at: &At) -> Result<EntrypointCounts, QueryError> {
        let (c, _) = self.step(at)?;
        let rows = self.entry_rows(c.seq as i64, None)?;
        let mut kinds: BTreeMap<&'static str, KindCount> = BTreeMap::new();
        for e in &rows {
            let name = match e.kind {
                EntrypointKind::Http => "http",
                EntrypointKind::Event => "event",
                EntrypointKind::Job => "job",
                EntrypointKind::Cli => "cli",
                EntrypointKind::Export => "export",
                EntrypointKind::Ui => "ui",
            };
            kinds
                .entry(name)
                .or_insert(KindCount {
                    kind: e.kind,
                    count: 0,
                })
                .count += 1;
        }
        let services: std::collections::BTreeSet<_> =
            rows.iter().filter_map(|e| e.service).collect();
        Ok(EntrypointCounts {
            total: rows.len() as u32,
            services: services.len() as u32,
            kinds: kinds.into_values().collect(),
        })
    }

    fn entry(
        &self,
        id: &str,
        at: &At,
    ) -> Result<(Row, u32, std::sync::Arc<Adjacency>), QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let row = self
            .entry_rows(c.seq as i64, Some(id))?
            .into_iter()
            .next()
            .ok_or_else(|| QueryError::NotFound {
                what: "no such entrypoint at that date".into(),
                action: "pass an id from …/entrypoints, and an at when it existed".into(),
            })?;
        Ok((row, c.seq as u32, g))
    }

    /// `GET …/entrypoints/{id}`: who calls it, with how sure each link is, and what it reaches by
    /// service. `env` limits both to the links that hold in one environment.
    pub fn entrypoint(
        &self,
        id: &str,
        at: &At,
        env: Option<&str>,
    ) -> Result<EntrypointDetail, QueryError> {
        let mask = self.env_mask(env)?;
        let (row, seq, g) = self.entry(id, at)?;
        let consumers = self.consumers(&row, i64::from(seq))?;
        let found = g
            .index
            .get(&row.node)
            .map(|&i| reached(&g, i, seq, mask))
            .unwrap_or_default();
        let mut by: BTreeMap<u32, u32> = BTreeMap::new();
        for &n in &found.nodes {
            let s = g.service[n as usize];
            if s != NONE {
                *by.entry(s).or_default() += 1;
            }
        }
        let mut services: Vec<ServiceReach> = by
            .into_iter()
            .map(|(s, nodes)| ServiceReach {
                service: NodeId(node_id(g.row[s as usize])),
                nodes,
            })
            .collect();
        services.sort_by_key(|s| std::cmp::Reverse(s.nodes));
        let nodes = found.nodes.len() as u32;
        Ok(EntrypointDetail {
            entrypoint: Self::wire(&row, consumers.len() as u32, nodes),
            consumers,
            reach: ReachSummary {
                nodes,
                services,
                truncated: found.truncated,
            },
        })
    }

    /// `GET …/entrypoints/{id}/trace`: the nodes the detail counts, as ids (Trace on the map). The
    /// entrypoint's own node comes first.
    pub fn entrypoint_trace(
        &self,
        id: &str,
        at: &At,
        env: Option<&str>,
    ) -> Result<Reach, QueryError> {
        let mask = self.env_mask(env)?;
        let (row, seq, g) = self.entry(id, at)?;
        let found = g
            .index
            .get(&row.node)
            .map(|&i| reached(&g, i, seq, mask))
            .unwrap_or_default();
        let nodes = std::iter::once(row.node)
            .chain(found.nodes.iter().map(|&n| g.row[n as usize]))
            .map(|r| NodeId(node_id(r)))
            .collect();
        Ok(Reach {
            nodes,
            links: Vec::new(),
            truncated: found.truncated,
        })
    }
}
