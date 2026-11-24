//! Node and link detail (SRV-08): the panel and inspector reads, each the Rust form of a
//! `lib/workspace.ts` helper (`metrics`, `series`, `neighbours`, `changesFor`, `ranking`,
//! `children`), answered from the generation's in-memory graph rather than a scan per request.
//!
//! A scope is a node and what lies below it: `parent` and `service` on every node make that one
//! comparison, never a recursive walk (SRV-08 scale). `workspace` names the whole workspace.

use std::collections::{BTreeSet, HashMap};

use cartograph_api::types::{
    Change, Commit, CommitRef, Confidence, ConfidenceLevel, DensityBin, Evidence, EvidenceKind,
    Interval, Lang, Layer, Lineage, LineageVia, Link, LinkChange, LinkDetail, Metrics, Node,
    NodeDetail, NodeKind, RankedNode,
};
use cartograph_core::{LinkId, NodeId};
use cartograph_store::rusqlite::{self, OptionalExtension};

use crate::adjacency::{Adjacency, Edge, Kind, NONE, Scope};
use crate::page::{Page, Paged};
use crate::resolve::{hex, link_id, node_id};
use crate::{At, QueryError, Snapshot, rfc3339};

/// The reference that names the whole workspace as a scope.
pub const WORKSPACE: &str = "workspace";
const DAY: i64 = 86_400;
/// `metrics.churn` and `ranking.delta` look back this far (`lib/workspace.ts`).
const CHURN_DAYS: i64 = 90;

/// A link's confidence level from its score (H2's bands); the server sends it, clients never
/// derive it (R20).
pub fn level(score: f32) -> ConfidenceLevel {
    if score >= 0.9 {
        ConfidenceLevel::Exact
    } else if score >= 0.6 {
        ConfidenceLevel::Likely
    } else {
        ConfidenceLevel::Guessed
    }
}

pub(crate) fn lang(path: &str) -> Lang {
    match path.rsplit_once('.').map_or("", |(_, e)| e) {
        "ts" | "tsx" | "mts" | "cts" => Lang::Ts,
        "js" | "jsx" | "mjs" | "cjs" => Lang::Js,
        "go" => Lang::Go,
        "py" => Lang::Py,
        "php" => Lang::Php,
        _ => Lang::Other,
    }
}

/// A resolved scope: the graph's view of it, and its node row (none for the workspace).
pub(crate) struct Resolved {
    pub scope: Scope,
    pub row: Option<i64>,
}

impl Snapshot<'_> {
    /// `at` as a step, and the instant `lib/workspace.ts` measures windows from: the date asked
    /// for, or the step's own time.
    pub(crate) fn step(&self, at: &At) -> Result<(CommitRef, i64), QueryError> {
        let c = self.resolve_at(at)?;
        let t = match at {
            At::Time(t) => *t,
            _ => self
                .conn()
                .prepare_cached("SELECT corrected_time FROM commits WHERE seq = ?1")?
                .query_row([c.seq as i64], |r| r.get(0))?,
        };
        Ok((c, t))
    }

    /// A node reference as a scope in `g`.
    pub(crate) fn scope(
        &self,
        g: &Adjacency,
        reference: &str,
        seq: u64,
    ) -> Result<Resolved, QueryError> {
        if reference == WORKSPACE {
            return Ok(Resolved {
                scope: Scope::All,
                row: None,
            });
        }
        let row = self.resolve_row(reference, seq)?;
        let n = g.index.get(&row).copied().ok_or(QueryError::Internal)?;
        Ok(Resolved {
            scope: Scope::Node(n),
            row: Some(row),
        })
    }

    pub(crate) fn resolve_row(&self, reference: &str, seq: u64) -> Result<i64, QueryError> {
        let id = self.resolve_node(reference, seq)?;
        id[1..].parse().map_err(|_| QueryError::Internal)
    }

    /// The step `seq` as the wire names it.
    pub(crate) fn commit_ref(&self, seq: u32) -> Result<CommitRef, QueryError> {
        let sha: Vec<u8> = self
            .conn()
            .prepare_cached("SELECT sha FROM commits WHERE seq = ?1")?
            .query_row([i64::from(seq)], |r| r.get(0))?;
        Ok(CommitRef {
            sha: hex(&sha),
            seq: u64::from(seq),
        })
    }

    /// One commit. Its message and author are repo text: shown as plain text, never as markup.
    pub(crate) fn commit(&self, seq: u32) -> Result<Commit, QueryError> {
        Ok(self
            .conn()
            .prepare_cached(
                "SELECT c.sha, c.corrected_time, c.author, r.name, c.summary
                 FROM commits c JOIN repo r ON r.id = c.repo WHERE c.seq = ?1",
            )?
            .query_row([i64::from(seq)], |r| {
                Ok(Commit {
                    sha: hex(&r.get::<_, Vec<u8>>(0)?),
                    seq: u64::from(seq),
                    date: rfc3339(r.get(1)?),
                    author: r.get(2)?,
                    repo: cartograph_core::RepoId(r.get(3)?),
                    message: r.get(4)?,
                })
            })?)
    }

    /// The wire form of node row `row`, with its path at `seq` (or its last one, when it isn't
    /// alive then).
    pub(crate) fn wire_node(&self, row: i64, seq: u64) -> Result<Node, QueryError> {
        type Raw = (
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let (kind, key, name, parent, service, file): Raw = self
            .conn()
            .prepare_cached(
                "SELECT kind, key, name, parent, service, file FROM node WHERE id = ?1",
            )?
            .query_row([row], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .optional()?
            .ok_or_else(|| QueryError::NotFound {
                what: "no such node".into(),
                action: "find the node with search, then pass its id".into(),
            })?;
        let (kind, path, lang_of) = match (kind.as_str(), file) {
            (_, Some(f)) => {
                let path: Option<String> = self
                    .conn()
                    .prepare_cached(
                        "SELECT path FROM file_path WHERE file = ?1
                         ORDER BY (from_seq <= ?2 AND (to_seq IS NULL OR to_seq > ?2)) DESC,
                                  from_seq DESC LIMIT 1",
                    )?
                    .query_row(rusqlite::params![f, seq as i64], |r| r.get(0))
                    .optional()?;
                let p = path.unwrap_or_default();
                (NodeKind::File, p.clone(), Some(lang(&p)))
            }
            ("module", None) => (
                NodeKind::Module,
                key.as_deref()
                    .and_then(|k| k.strip_prefix("module:"))
                    .unwrap_or_default()
                    .to_owned(),
                None,
            ),
            (_, None) => (
                // A library is a service that deploys nothing (C6); the wire has one kind for both.
                NodeKind::Service,
                key.as_deref()
                    .and_then(|k| k.splitn(3, ':').nth(2))
                    .unwrap_or_default()
                    .to_owned(),
                None,
            ),
        };
        let name = match kind {
            NodeKind::File => path.rsplit('/').next().unwrap_or(&path).to_owned(),
            _ => name.unwrap_or_else(|| path.clone()),
        };
        Ok(Node {
            id: NodeId(node_id(row)),
            name,
            kind,
            parent: parent.map(|p| NodeId(node_id(p))),
            service: service.map(|s| NodeId(node_id(s))),
            path,
            summary: None,
            lang: lang_of,
        })
    }

    /// The wire form of one edge (a link over one interval).
    pub(crate) fn wire_link(&self, g: &Adjacency, e: &Edge) -> Result<Link, QueryError> {
        Ok(Link {
            id: LinkId(link_id(e.link)),
            from: NodeId(node_id(g.row[e.src as usize])),
            to: NodeId(node_id(g.row[e.dst as usize])),
            kind: e.kind,
            added: self.commit_ref(e.from)?,
            removed: if e.to == NONE {
                None
            } else {
                Some(self.commit_ref(e.to)?)
            },
            confidence: Confidence {
                level: level(e.score),
                score: e.score,
                evidence: Vec::new(),
            },
        })
    }

    fn workspace_node(&self) -> Result<Node, QueryError> {
        let name: Option<String> = self
            .conn()
            .prepare_cached("SELECT name FROM repo WHERE removed = 0 ORDER BY id LIMIT 1")?
            .query_row([], |r| r.get(0))
            .optional()?;
        Ok(Node {
            id: NodeId(WORKSPACE.into()),
            name: name.unwrap_or_else(|| WORKSPACE.into()),
            kind: NodeKind::Workspace,
            parent: None,
            service: None,
            path: String::new(),
            summary: None,
            lang: None,
        })
    }

    /// `GET …/nodes/{id}`: the node, its metrics, and the steps it appeared and went.
    pub fn node_detail(&self, reference: &str, at: &At) -> Result<NodeDetail, QueryError> {
        let (c, t) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let metrics = metrics(&g, r.scope, c.seq as u32, t);
        let Some(row) = r.row else {
            return Ok(NodeDetail {
                node: self.workspace_node()?,
                metrics,
                owners: Vec::new(),
                added: None,
                removed: None,
            });
        };
        let (first, open, last): (Option<i64>, i64, Option<i64>) = self
            .conn()
            .prepare_cached(
                "SELECT min(from_seq), count(*) FILTER (WHERE to_seq IS NULL), max(to_seq) FROM (
                   SELECT fp.from_seq, fp.to_seq FROM node n JOIN file_path fp ON fp.file = n.file
                   WHERE n.id = ?1
                   UNION ALL SELECT from_seq, to_seq FROM node_span WHERE node = ?1)",
            )?
            .query_row([row], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(NodeDetail {
            node: self.wire_node(row, c.seq)?,
            metrics,
            owners: self.owners_of_node(row, c.seq as u32)?,
            added: first.map(|s| self.commit_ref(s as u32)).transpose()?,
            removed: match (open, last) {
                (0, Some(s)) => Some(self.commit_ref(s as u32)?),
                _ => None,
            },
        })
    }

    /// `GET …/nodes/{id}/children`: the nodes one level down that are alive at `at`, services by
    /// name, then modules, then files by path.
    pub fn children(
        &self,
        reference: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<Node>, QueryError> {
        let (c, _) = self.step(at)?;
        let parent = if reference == WORKSPACE {
            None
        } else {
            Some(self.resolve_row(reference, c.seq)?)
        };
        let rows: Vec<i64> = self
            .conn()
            .prepare_cached(
                "SELECT n.id FROM node n
                 WHERE (n.parent = ?1 OR (?1 IS NULL AND n.parent IS NULL))
                   AND (EXISTS (SELECT 1 FROM file_path fp WHERE fp.file = n.file
                          AND fp.from_seq <= ?2 AND (fp.to_seq IS NULL OR fp.to_seq > ?2))
                     OR EXISTS (SELECT 1 FROM node_span ns WHERE ns.node = n.id
                          AND ns.from_seq <= ?2 AND (ns.to_seq IS NULL OR ns.to_seq > ?2)))
                 ORDER BY n.id",
            )?
            .query_map(rusqlite::params![parent, c.seq as i64], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut nodes = rows
            .iter()
            .map(|&r| self.wire_node(r, c.seq))
            .collect::<Result<Vec<_>, _>>()?;
        let rank = |n: &Node| match n.kind {
            NodeKind::Service => 0,
            NodeKind::Module => 1,
            _ => 2,
        };
        nodes.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.path.cmp(&b.path)));
        Ok(page.take(nodes))
    }

    /// `GET …/nodes/{id}/metrics` (`lib/workspace.ts` `metrics`).
    pub fn metrics(&self, reference: &str, at: &At) -> Result<Metrics, QueryError> {
        let (c, t) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        Ok(metrics(&g, r.scope, c.seq as u32, t))
    }

    /// `GET …/nodes/{id}/series`: `months` monthly bins ending with the month of `at`, each with
    /// the links touching the scope that were added and removed in it, and the commits that did
    /// so. One pass over the interval end points, not a metrics call per month (SRV-08). The
    /// live count at any month is the count at `at` minus the later bins' net change.
    pub fn series(
        &self,
        reference: &str,
        at: &At,
        months: usize,
    ) -> Result<Vec<DensityBin>, QueryError> {
        let (c, t) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let months = months.clamp(1, 240);
        let end = crate::timeline::month_start(t);
        let starts: Vec<i64> = (0..months)
            .rev()
            .map(|i| crate::timeline::add_months(end, -(i as i64)))
            .collect();
        let mut bins: Vec<(BTreeSet<u32>, u32, u32)> = vec![(BTreeSet::new(), 0, 0); months];
        let bin_of = |seq: u32| -> Option<usize> {
            let at = g.time[seq as usize];
            if at > t || at < starts[0] {
                return None;
            }
            Some(starts.partition_point(|&s| s <= at) - 1)
        };
        for e in &g.edges {
            if !(g.within(e.src, r.scope) || g.within(e.dst, r.scope)) {
                continue;
            }
            if let Some(b) = bin_of(e.from) {
                bins[b].0.insert(e.from);
                bins[b].1 += 1;
            }
            if e.to != NONE
                && let Some(b) = bin_of(e.to)
            {
                bins[b].0.insert(e.to);
                bins[b].2 += 1;
            }
        }
        Ok(starts
            .iter()
            .zip(bins)
            .map(|(&s, (commits, added, removed))| DensityBin {
                start: rfc3339(s),
                commits: commits.len() as u32,
                links_added: added,
                links_removed: removed,
                by_repo: Vec::new(),
            })
            .collect())
    }

    /// `GET …/nodes/{id}/neighbors`: the live links crossing the scope's boundary, in either
    /// direction, by the node on the other end (`lib/workspace.ts` `neighbours`).
    pub fn neighbors(
        &self,
        reference: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<Link>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let seq = c.seq as u32;
        let mut hits: Vec<(i64, &Edge)> = g
            .edges
            .iter()
            .filter(|e| e.live(seq))
            .filter_map(
                |e| match (g.within(e.src, r.scope), g.within(e.dst, r.scope)) {
                    (true, false) => Some((g.row[e.dst as usize], e)),
                    (false, true) => Some((g.row[e.src as usize], e)),
                    _ => None,
                },
            )
            .collect();
        hits.sort_by_key(|(other, e)| (*other, e.link));
        let p = page.take(hits);
        Ok(Paged {
            rows: p
                .rows
                .iter()
                .map(|(_, e)| self.wire_link(&g, e))
                .collect::<Result<_, _>>()?,
            next_cursor: p.next_cursor,
        })
    }

    /// `GET …/nodes/{id}/changes`: each link touching the scope that was added or removed at or
    /// before `at`, newest first (`lib/workspace.ts` `changesFor`).
    pub fn changes(
        &self,
        reference: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<LinkChange>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let seq = c.seq as u32;
        let mut rows: Vec<(u32, Change, &Edge)> = Vec::new();
        for e in &g.edges {
            if !(g.within(e.src, r.scope) || g.within(e.dst, r.scope)) {
                continue;
            }
            if e.from <= seq {
                rows.push((e.from, Change::Added, e));
            }
            if e.to != NONE && e.to <= seq {
                rows.push((e.to, Change::Removed, e));
            }
        }
        rows.sort_by_key(|(s, ch, e)| (std::cmp::Reverse(*s), *ch == Change::Added, e.link));
        let p = page.take(rows);
        let mut commits: HashMap<u32, Commit> = HashMap::new();
        let mut out = Vec::with_capacity(p.rows.len());
        for (s, change, e) in p.rows {
            let commit = match commits.get(&s) {
                Some(c) => c.clone(),
                None => {
                    let c = self.commit(s)?;
                    commits.insert(s, c.clone());
                    c
                }
            };
            out.push(LinkChange {
                commit,
                link: self.wire_link(&g, e)?,
                change,
            });
        }
        Ok(Paged {
            rows: out,
            next_cursor: p.next_cursor,
        })
    }

    /// `GET …/nodes/{id}/ranking`: files in the scope by how many live links point at them, then
    /// how many they use; files with neither are left out (`lib/workspace.ts` `ranking`).
    pub fn ranking(
        &self,
        reference: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<RankedNode>, QueryError> {
        let (c, t) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let (seq, ago) = (c.seq as u32, g.seq_at(t - CHURN_DAYS * DAY));
        let mut rows: Vec<(u32, u32, u32, i32)> = g
            .members(r.scope)
            .into_iter()
            .filter(|&n| g.kind[n as usize] == Kind::File)
            .filter_map(|n| {
                let used_by = g.inbound(n).filter(|e| e.live(seq)).count() as u32;
                let uses = g.out(n).filter(|e| e.live(seq)).count() as u32;
                let then = ago.map_or(0, |a| g.inbound(n).filter(|e| e.live(a)).count()) as i32;
                (used_by + uses > 0).then_some((n, used_by, uses, used_by as i32 - then))
            })
            .collect();
        rows.sort_by_key(|&(n, used_by, uses, _)| {
            (
                std::cmp::Reverse(used_by),
                std::cmp::Reverse(uses),
                g.row[n as usize],
            )
        });
        let p = page.take(rows);
        Ok(Paged {
            rows: p
                .rows
                .into_iter()
                .map(|(n, used_by, uses, delta)| {
                    Ok(RankedNode {
                        node: self.wire_node(g.row[n as usize], c.seq)?,
                        used_by,
                        uses,
                        delta,
                    })
                })
                .collect::<Result<_, QueryError>>()?,
            next_cursor: p.next_cursor,
        })
    }

    /// `GET …/nodes/{id}/history`: the paths a file had, oldest first, and how it got each one
    /// (board G10); a service's or module's spans.
    pub fn history(&self, reference: &str) -> Result<Vec<Lineage>, QueryError> {
        let (c, _) = self.step(&At::Now)?;
        let row = self.resolve_row(reference, c.seq)?;
        type Raw = (String, i64, Option<i64>, String, Option<i64>);
        let raw: Vec<Raw> = self
            .conn()
            .prepare_cached(
                "SELECT fp.path, fp.from_seq, fp.to_seq, fp.via, fp.similarity
                 FROM node n JOIN file_path fp ON fp.file = n.file WHERE n.id = ?1
                 UNION ALL
                 SELECT coalesce(substr(n.key, instr(n.key, ':') + 1), ''), ns.from_seq, ns.to_seq,
                        'add', NULL
                 FROM node n JOIN node_span ns ON ns.node = n.id WHERE n.id = ?1
                 ORDER BY 2",
            )?
            .query_map([row], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<_, _>>()?;
        raw.into_iter()
            .map(|(path, from, to, via, similarity)| {
                Ok(Lineage {
                    path,
                    from: self.commit_ref(from as u32)?,
                    to: to.map(|t| self.commit_ref(t as u32)).transpose()?,
                    via: match via.as_str() {
                        "rename-exact" => LineageVia::RenameExact,
                        "rename-similar" => LineageVia::RenameSimilar,
                        _ => LineageVia::Add,
                    },
                    similarity: similarity.map(|s| s.clamp(0, 100) as u8),
                })
            })
            .collect()
    }

    /// `GET …/nodes/{id}/symbols`.
    // ponytail: no symbols are indexed until stage 5 (XS-25); the route answers an empty list so
    // the panel can render its empty state against the real API.
    pub fn symbols(&self, reference: &str) -> Result<Vec<Node>, QueryError> {
        let (c, _) = self.step(&At::Now)?;
        self.resolve_row(reference, c.seq)?;
        Ok(Vec::new())
    }

    /// `GET …/nodes/{id}/reading-order`: the scope's live files, each after the files in the scope
    /// it depends on (a cycle is broken at its most used file), most used first among equals.
    pub fn reading_order(
        &self,
        reference: &str,
        at: &At,
        page: Page,
    ) -> Result<Paged<NodeId>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let r = self.scope(&g, reference, c.seq)?;
        let seq = c.seq as u32;
        let files: Vec<u32> = g
            .members(r.scope)
            .into_iter()
            .filter(|&n| g.kind[n as usize] == Kind::File)
            .filter(|&n| g.out(n).any(|e| e.live(seq)) || g.inbound(n).any(|e| e.live(seq)))
            .collect();
        let inside: std::collections::HashSet<u32> = files.iter().copied().collect();
        let fan_in = |n: u32| g.inbound(n).filter(|e| e.live(seq)).count();
        // Kahn's order over "depends on" edges inside the scope: what a file imports comes first.
        let mut waiting: HashMap<u32, usize> = files
            .iter()
            .map(|&n| {
                let deps = g
                    .out(n)
                    .filter(|e| e.live(seq) && inside.contains(&e.dst) && e.dst != n)
                    .map(|e| e.dst)
                    .collect::<BTreeSet<_>>()
                    .len();
                (n, deps)
            })
            .collect();
        let mut order = Vec::with_capacity(files.len());
        let mut done = std::collections::HashSet::new();
        while order.len() < files.len() {
            let mut ready: Vec<u32> = waiting
                .iter()
                .filter(|&(n, &d)| d == 0 && !done.contains(n))
                .map(|(&n, _)| n)
                .collect();
            if ready.is_empty() {
                // A cycle: start it at its most used file.
                let n = *waiting
                    .keys()
                    .filter(|n| !done.contains(*n))
                    .max_by_key(|&&n| (fan_in(n), std::cmp::Reverse(g.row[n as usize])))
                    .expect("files are left");
                ready.push(n);
            }
            ready.sort_by_key(|&n| (std::cmp::Reverse(fan_in(n)), g.row[n as usize]));
            for n in ready {
                done.insert(n);
                order.push(n);
                for e in g
                    .inbound(n)
                    .filter(|e| e.live(seq) && inside.contains(&e.src))
                {
                    if let Some(w) = waiting.get_mut(&e.src)
                        && *w > 0
                        && !done.contains(&e.src)
                    {
                        *w -= 1;
                    }
                }
            }
        }
        let p = page.take(order);
        Ok(Paged {
            rows: p
                .rows
                .into_iter()
                .map(|n| NodeId(node_id(g.row[n as usize])))
                .collect(),
            next_cursor: p.next_cursor,
        })
    }

    /// `GET …/links/{id}`: the link over its interval live at `at` (else its last one before
    /// `at`), the layer that won and its evidence. Evidence notes are fixed text; the file is the
    /// importer's path.
    pub fn link_detail(&self, id: &str, at: &At) -> Result<LinkDetail, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let missing = || QueryError::NotFound {
            what: "no such link".into(),
            action: "take link ids from a graph slice or a node's neighbors".into(),
        };
        let row: i64 = id
            .strip_prefix('l')
            .filter(|d| (1..=18).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse().ok())
            .ok_or_else(missing)?;
        let start = g.edges.partition_point(|e| e.link < row);
        let all: Vec<&Edge> = g.edges[start..]
            .iter()
            .take_while(|e| e.link == row)
            .collect();
        let seq = c.seq as u32;
        let e = all
            .iter()
            .find(|e| e.live(seq))
            .or_else(|| all.iter().rev().find(|e| e.from <= seq))
            .or(all.first())
            .copied()
            .ok_or_else(missing)?;
        let mut link = self.wire_link(&g, e)?;
        let file = self.wire_node(g.row[e.src as usize], c.seq)?.path;
        link.confidence.evidence = vec![Evidence {
            kind: EvidenceKind::Parsed,
            note: "an import statement in the source file".into(),
            file: Some(file),
            line: None,
        }];
        Ok(LinkDetail {
            link,
            layer: Layer::Base,
            interval: Interval {
                from_seq: u64::from(e.from),
                to_seq: (e.to != NONE).then_some(u64::from(e.to)),
            },
            replaced_by: None,
            correction_id: None,
        })
    }

    /// For `GET …/source`: the repo folder and blob id of a file node at a commit, read from the
    /// index only, so the route can never name an object or a path itself (SRV-08 security).
    pub fn source_blob(
        &self,
        reference: &str,
        commit: &At,
    ) -> Result<(std::path::PathBuf, Vec<u8>), QueryError> {
        let c = self.resolve_at(commit)?;
        let row = self.resolve_row(reference, c.seq)?;
        let found: Option<(String, Vec<u8>)> = self
            .conn()
            .prepare_cached(
                "SELECT r.path, v.blob FROM node n JOIN file f ON f.id = n.file
                 JOIN repo r ON r.id = f.repo JOIN file_version v ON v.file = f.id
                 WHERE n.id = ?1 AND v.from_seq <= ?2 AND (v.to_seq IS NULL OR v.to_seq > ?2)",
            )?
            .query_row(rusqlite::params![row, c.seq as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (repo, blob) = found.ok_or_else(|| QueryError::NotFound {
            what: "the node has no file content at that commit".into(),
            action: "pass a file node and a commit at which it exists".into(),
        })?;
        Ok((repo.into(), blob))
    }
}

/// `lib/workspace.ts` `metrics`: live links with an end in the scope, those entering and leaving
/// it, those between services, and the interval end points in the 90 days up to `t`.
pub(crate) fn metrics(g: &Adjacency, scope: Scope, seq: u32, t: i64) -> Metrics {
    let mut m = Metrics {
        links: 0,
        fan_in: 0,
        fan_out: 0,
        cross: 0,
        churn: 0,
    };
    let recent = |s: u32| {
        let d = g.time[s as usize];
        d <= t && t - d < CHURN_DAYS * DAY
    };
    for e in &g.edges {
        let (fi, ti) = (g.within(e.src, scope), g.within(e.dst, scope));
        if !fi && !ti {
            continue;
        }
        m.churn += u32::from(recent(e.from)) + u32::from(e.to != NONE && recent(e.to));
        if !e.live(seq) {
            continue;
        }
        m.links += 1;
        m.fan_in += u32::from(ti && !fi);
        m.fan_out += u32::from(fi && !ti);
        m.cross += u32::from(g.service[e.src as usize] != g.service[e.dst as usize]);
    }
    m
}
