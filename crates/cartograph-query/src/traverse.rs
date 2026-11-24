//! Traversals (SRV-11): impact, path and reach over the generation's in-memory graph, shared by
//! HTTP, MCP and Ask. Every walk stops at 50k visited nodes, 12 hops or 5 s, and says so with
//! `truncated`. Visited sets are bitsets.
//!
//! A path's confidence is its weakest link; a node's impact level is the best such confidence
//! over the paths that reach it, found as three walks over links at or above each level's floor.
//!
//! ponytail: `ref` and `working_tree` inputs need branch overlays and the in-memory parse
//! (IDX-24, phase 5); they are refused by name until then. `stories` needs entrypoints (XS
//! items) and answers an empty list.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use cartograph_api::types::{ImpactResult, PathDirection, PathResult, Reach};
use cartograph_core::{LinkId, NodeId};
use cartograph_store::rusqlite;

use crate::adjacency::{Adjacency, Edge, Kind, NONE};
use crate::detail::level;
use crate::resolve::{link_id, node_id};
use crate::{At, QueryError, Snapshot};

pub const MAX_VISITED: usize = 50_000;
pub const MAX_DEPTH: usize = 12;
pub const DEADLINE: Duration = Duration::from_secs(5);
/// Paths `path` returns at most.
const MAX_PATHS: usize = 3;

struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    /// Sets bit `i`; true when it was clear.
    fn insert(&mut self, i: u32) -> bool {
        let (w, b) = (i as usize / 64, 1u64 << (i % 64));
        let was = self.0[w] & b == 0;
        self.0[w] |= b;
        was
    }
}

/// What a walk found, in the order it found it (nearest first).
pub(crate) struct Walk {
    pub nodes: Vec<u32>,
    pub links: Vec<i64>,
    pub truncated: bool,
}

/// Breadth-first from `seeds` over links live at `seq` with a score of at least `floor`,
/// following `step` (inbound edges for dependents, outbound for dependencies). Seeds aren't in the
/// answer.
fn walk<'g, I: Iterator<Item = &'g Edge>>(
    g: &'g Adjacency,
    seeds: &[u32],
    seq: u32,
    floor: f32,
    max_depth: usize,
    step: impl Fn(u32) -> I,
    other: impl Fn(&Edge) -> u32,
) -> Walk {
    let start = Instant::now();
    let mut seen = Bits::new(g.row.len());
    let mut out = Walk {
        nodes: Vec::new(),
        links: Vec::new(),
        truncated: false,
    };
    let mut frontier: Vec<u32> = seeds.iter().copied().filter(|&s| seen.insert(s)).collect();
    let mut visited = frontier.len();
    'outer: for depth in 0..max_depth {
        let mut next = Vec::new();
        for (i, &n) in frontier.iter().enumerate() {
            if i % 1024 == 0 && start.elapsed() > DEADLINE {
                out.truncated = true;
                break 'outer;
            }
            for e in step(n) {
                if !e.live(seq) || e.score < floor {
                    continue;
                }
                out.links.push(e.link);
                let m = other(e);
                if seen.insert(m) {
                    visited += 1;
                    if visited > MAX_VISITED {
                        out.truncated = true;
                        break 'outer;
                    }
                    out.nodes.push(m);
                    next.push(m);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        if depth + 1 == max_depth {
            out.truncated = true;
        }
        frontier = next;
    }
    out
}

/// Everything that depends on `seeds` at `seq` over links scoring at least `floor`.
pub(crate) fn dependents(g: &Adjacency, seeds: &[u32], seq: u32, floor: f32) -> Walk {
    walk(g, seeds, seq, floor, MAX_DEPTH, |n| g.inbound(n), |e| e.src)
}

/// What a request named, before it is walked.
#[derive(Debug, Clone)]
pub enum ImpactInput {
    /// Node references (ids, paths, names): each with everything below it.
    Nodes(Vec<String>),
    /// The files changed after this point, up to `at`.
    Since(At),
    /// A branch or a commit range.
    Ref(String),
    /// The uncommitted changes.
    WorkingTree,
}

/// Up to `n` of `rows` as ids.
fn ids(g: &Adjacency, rows: &[u32]) -> Vec<NodeId> {
    rows.iter()
        .map(|&n| NodeId(node_id(g.row[n as usize])))
        .collect()
}

impl Snapshot<'_> {
    /// `GET …/impact?node=` and `POST …/impact`: what depends on the input, as exact, likely and
    /// guessed, nearest first, and the services they are in (`lib/ask.ts` `impact`).
    pub fn impact(&self, input: &ImpactInput, at: &At) -> Result<ImpactResult, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let seq = c.seq as u32;
        let seeds: Vec<u32> = match input {
            ImpactInput::Nodes(refs) => {
                if refs.is_empty() || refs.len() > 1000 {
                    return Err(QueryError::bad(
                        "impact needs 1 to 1,000 paths or nodes",
                        "each one is a node id, a path or a name",
                        "pass the paths you changed",
                    ));
                }
                let mut set = BTreeSet::new();
                for r in refs {
                    let s = self.scope(&g, r, c.seq)?.scope;
                    set.extend(g.members(s));
                }
                set.into_iter().collect()
            }
            ImpactInput::Since(since) => self.changed_since(&g, since, c.seq)?,
            ImpactInput::Ref(_) | ImpactInput::WorkingTree => {
                return Err(QueryError::bad(
                    "impact of a ref or the working tree isn't available yet",
                    "it reads branch overlays and parses uncommitted files in memory, which arrive \
                     with IDX-24 in phase 5",
                    "pass the changed paths, or since: a commit or date on the indexed branch",
                ));
            }
        };
        // The three walks are independent, so they run side by side.
        let walks = std::thread::scope(|t| {
            [0.9f32, 0.6, 0.3]
                .map(|floor| {
                    let (g, seeds) = (&g, &seeds);
                    t.spawn(move || dependents(g, seeds, seq, floor))
                })
                .map(|h| h.join().expect("an impact walk panicked"))
        });
        let mut placed = Bits::new(g.row.len());
        let mut levels: [Vec<u32>; 3] = Default::default();
        for (i, w) in walks.iter().enumerate() {
            for &n in &w.nodes {
                if placed.insert(n) {
                    levels[i].push(n);
                }
            }
        }
        let mut services: Vec<u32> = levels
            .iter()
            .flatten()
            .map(|&n| match g.kind[n as usize] {
                Kind::Service | Kind::Library => n,
                _ => g.service[n as usize],
            })
            .filter(|&s| s != NONE)
            .collect();
        services.sort_unstable();
        services.dedup();
        let [exact, likely, guessed] = levels;
        Ok(ImpactResult {
            changed: ids(&g, &seeds),
            exact: ids(&g, &exact),
            likely: ids(&g, &likely),
            guessed: ids(&g, &guessed),
            services: ids(&g, &services),
            truncated: walks.iter().any(|w| w.truncated),
        })
    }

    /// File nodes whose content or path changed after `since`, up to step `now`.
    fn changed_since(&self, g: &Adjacency, since: &At, now: u64) -> Result<Vec<u32>, QueryError> {
        let from = self.resolve_at(since)?.seq as i64;
        let rows: Vec<i64> = self
            .conn()
            .prepare_cached(
                "SELECT DISTINCT n.id FROM node n WHERE n.file IN (
                   SELECT file FROM file_version WHERE from_seq > ?1 AND from_seq <= ?2
                   UNION SELECT file FROM file_version WHERE to_seq > ?1 AND to_seq <= ?2
                   UNION SELECT file FROM file_path WHERE from_seq > ?1 AND from_seq <= ?2)",
            )?
            .query_map(rusqlite::params![from, now as i64], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if rows.len() > MAX_VISITED {
            return Err(QueryError::TooLarge {
                why: format!("more than {MAX_VISITED} files changed in that range"),
                narrower: "pass a later since".into(),
            });
        }
        Ok(rows
            .iter()
            .filter_map(|r| g.index.get(r).copied())
            .collect())
    }

    /// `GET …/path`: up to three shortest chains of live links from `from` to `to`, following
    /// dependency direction first, then the reverse, then either (`lib/ask.ts` `path`).
    pub fn path(
        &self,
        from: &str,
        to: &str,
        max_hops: usize,
        at: &At,
    ) -> Result<Vec<PathResult>, QueryError> {
        if !(1..=MAX_DEPTH).contains(&max_hops) {
            return Err(QueryError::bad(
                "max_hops is out of range",
                "a path has 1 to 12 hops",
                "pass max_hops from 1 to 12",
            ));
        }
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let seq = c.seq as u32;
        let a = g.members(self.scope(&g, from, c.seq)?.scope);
        let b = g.members(self.scope(&g, to, c.seq)?.scope);
        let target: std::collections::HashSet<u32> = b.iter().copied().collect();
        if a.iter().any(|n| target.contains(n)) {
            // The two overlap: the path is empty, as `lib/ask.ts` answers.
            return Ok(vec![PathResult {
                hops: Vec::new(),
                level: cartograph_api::types::ConfidenceLevel::Exact,
                direction: PathDirection::Forward,
            }]);
        }
        for direction in [
            PathDirection::Forward,
            PathDirection::Backward,
            PathDirection::Mixed,
        ] {
            let chains = shortest(&g, &a, &target, seq, max_hops, direction);
            if !chains.is_empty() {
                return chains
                    .into_iter()
                    .map(|hops| {
                        let weakest = hops.iter().map(|e| e.score).fold(1.0f32, f32::min);
                        Ok(PathResult {
                            hops: hops
                                .iter()
                                .map(|e| self.wire_link(&g, e))
                                .collect::<Result<_, QueryError>>()?,
                            level: level(weakest),
                            direction,
                        })
                    })
                    .collect();
            }
        }
        Ok(Vec::new())
    }

    /// `GET …/reach`: what `node` (and what lies below it) reaches over live links, following
    /// dependencies (`out`, the default, as "Trace on the map" does), dependents (`in`) or both.
    pub fn reach(
        &self,
        node: &str,
        direction: &str,
        depth: usize,
        at: &At,
    ) -> Result<Reach, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let seq = c.seq as u32;
        let seeds = g.members(self.scope(&g, node, c.seq)?.scope);
        let depth = depth.clamp(1, MAX_DEPTH);
        let w = match direction {
            "" | "out" => walk(&g, &seeds, seq, 0.0, depth, |n| g.out(n), |e| e.dst),
            "in" => walk(&g, &seeds, seq, 0.0, depth, |n| g.inbound(n), |e| e.src),
            "both" => both(&g, &seeds, seq, depth),
            _ => {
                return Err(QueryError::bad(
                    "direction isn't out, in or both",
                    "reach follows dependencies (out), dependents (in) or both",
                    "for example direction=out",
                ));
            }
        };
        let mut nodes = seeds.clone();
        nodes.extend(&w.nodes);
        let mut links = w.links;
        links.sort_unstable();
        links.dedup();
        Ok(Reach {
            nodes: ids(&g, &nodes),
            links: links.into_iter().map(|l| LinkId(link_id(l))).collect(),
            truncated: w.truncated,
        })
    }

    /// `GET …/stories`.
    pub fn stories(&self, node: &str, at: &At) -> Result<Vec<PathResult>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        self.scope(&g, node, c.seq)?;
        Ok(Vec::new())
    }
}

/// Reach in both directions: each edge leads to whichever end isn't the node it was reached from.
fn both(g: &Adjacency, seeds: &[u32], seq: u32, depth: usize) -> Walk {
    let start = Instant::now();
    let mut seen = Bits::new(g.row.len());
    let mut out = Walk {
        nodes: Vec::new(),
        links: Vec::new(),
        truncated: false,
    };
    let mut frontier: Vec<u32> = seeds.iter().copied().filter(|&s| seen.insert(s)).collect();
    for d in 0..depth {
        let mut next = Vec::new();
        for &n in &frontier {
            if start.elapsed() > DEADLINE || out.nodes.len() > MAX_VISITED {
                out.truncated = true;
                return out;
            }
            for (e, m) in g
                .out(n)
                .map(|e| (e, e.dst))
                .chain(g.inbound(n).map(|e| (e, e.src)))
            {
                if !e.live(seq) {
                    continue;
                }
                out.links.push(e.link);
                if seen.insert(m) {
                    out.nodes.push(m);
                    next.push(m);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        if d + 1 == depth {
            out.truncated = true;
        }
        frontier = next;
    }
    out
}

/// Up to [`MAX_PATHS`] shortest chains from any node of `from` to any node of `to`, each ending
/// with a different last hop. Breadth-first, so every chain is as short as the first one found or
/// one hop longer at most.
fn shortest<'g>(
    g: &'g Adjacency,
    from: &[u32],
    to: &std::collections::HashSet<u32>,
    seq: u32,
    max_hops: usize,
    direction: PathDirection,
) -> Vec<Vec<&'g Edge>> {
    let start = Instant::now();
    let mut prev: HashMap<u32, Option<&Edge>> = HashMap::new();
    let mut queue: VecDeque<(u32, usize)> = VecDeque::new();
    for &n in from {
        if prev.insert(n, None).is_none() {
            queue.push_back((n, 0));
        }
    }
    let chain = |prev: &HashMap<u32, Option<&'g Edge>>, mut n: u32| {
        let mut hops = Vec::new();
        while let Some(Some(e)) = prev.get(&n) {
            hops.push(*e);
            n = if e.dst == n { e.src } else { e.dst };
        }
        hops.reverse();
        hops
    };
    let mut found: Vec<Vec<&Edge>> = Vec::new();
    let mut shortest_len = usize::MAX;
    while let Some((n, d)) = queue.pop_front() {
        if d >= max_hops
            || d > shortest_len
            || prev.len() > MAX_VISITED
            || start.elapsed() > DEADLINE
        {
            break;
        }
        let steps: Vec<(&Edge, u32)> = match direction {
            PathDirection::Forward => g.out(n).map(|e| (e, e.dst)).collect(),
            PathDirection::Backward => g.inbound(n).map(|e| (e, e.src)).collect(),
            PathDirection::Mixed => g
                .out(n)
                .map(|e| (e, e.dst))
                .chain(g.inbound(n).map(|e| (e, e.src)))
                .collect(),
        };
        for (e, m) in steps {
            if !e.live(seq) {
                continue;
            }
            // A target reached from outside the start set closes a chain; each edge into a
            // target is a different last hop.
            if to.contains(&m) && !matches!(prev.get(&m), Some(None)) {
                let mut hops = chain(&prev, n);
                hops.push(e);
                shortest_len = shortest_len.min(hops.len());
                if hops.len() <= shortest_len + 1 {
                    found.push(hops);
                }
                if found.len() == MAX_PATHS {
                    return found;
                }
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(v) = prev.entry(m) {
                v.insert(Some(e));
                queue.push_back((m, d + 1));
            }
        }
    }
    found
}

/// What `neighbors` (MCP) walks: which way, how far and over which links.
#[derive(Debug, Clone)]
pub struct NeighborQuery {
    /// Follow links into the node (its dependents, callers).
    pub inbound: bool,
    /// Follow links out of it (its dependencies, callees).
    pub outbound: bool,
    /// 1 to 5 hops.
    pub depth: usize,
    /// None keeps every kind.
    pub kinds: Option<Vec<cartograph_api::types::LinkKind>>,
    /// Only links between two services.
    pub cross_service: bool,
    pub min_score: f32,
}

/// One link a neighbourhood walk crossed.
#[derive(Debug, Clone)]
pub struct Neighbor {
    pub link: cartograph_api::types::Link,
    /// The end the walk reached through this link.
    pub node: NodeId,
    /// 1 for a link touching the node itself.
    pub depth: u8,
    /// The link points into the walk's side (a dependent).
    pub inbound: bool,
}

impl Snapshot<'_> {
    /// The links around `node` (and what lies below it) out to `depth` hops, nearest first. Links
    /// inside the node's own scope are left out, as `neighbours` does.
    pub fn neighborhood(
        &self,
        node: &str,
        q: &NeighborQuery,
        at: &At,
        page: crate::Page,
    ) -> Result<crate::Paged<Neighbor>, QueryError> {
        let (c, _) = self.step(at)?;
        let g = self.adjacency()?;
        let seq = c.seq as u32;
        let scope = self.scope(&g, node, c.seq)?.scope;
        let seeds = g.members(scope);
        let depth = q.depth.clamp(1, 5);
        let start = Instant::now();
        let mut seen = Bits::new(g.row.len());
        for &s in &seeds {
            seen.insert(s);
        }
        let keep = |e: &Edge| {
            e.live(seq)
                && e.score >= q.min_score
                && q.kinds.as_ref().is_none_or(|k| k.contains(&e.kind))
                && (!q.cross_service || g.service[e.src as usize] != g.service[e.dst as usize])
                && !(g.within(e.src, scope) && g.within(e.dst, scope))
        };
        let mut rows: Vec<(&Edge, u32, u8, bool)> = Vec::new();
        let mut frontier = seeds;
        let mut truncated = false;
        // ponytail: a walk cut by a cap returns what it found; the page says nothing about it.
        for d in 1..=depth {
            let mut next = Vec::new();
            for &n in &frontier {
                if start.elapsed() > DEADLINE || rows.len() > MAX_VISITED {
                    truncated = true;
                    break;
                }
                let ins = g.inbound(n).filter(|_| q.inbound).map(|e| (e, e.src, true));
                let outs = g.out(n).filter(|_| q.outbound).map(|e| (e, e.dst, false));
                for (e, m, inbound) in ins.chain(outs) {
                    if !keep(e) {
                        continue;
                    }
                    rows.push((e, m, d as u8, inbound));
                    if seen.insert(m) {
                        next.push(m);
                    }
                }
            }
            if next.is_empty() || truncated {
                break;
            }
            frontier = next;
        }
        let p = page.take(rows);
        let rows = p
            .rows
            .into_iter()
            .map(|(e, m, depth, inbound)| {
                Ok(Neighbor {
                    link: self.wire_link(&g, e)?,
                    node: NodeId(node_id(g.row[m as usize])),
                    depth,
                    inbound,
                })
            })
            .collect::<Result<_, QueryError>>()?;
        Ok(crate::Paged {
            rows,
            next_cursor: p.next_cursor,
        })
    }
}
