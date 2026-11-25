//! `graph(GraphQuery) -> GraphSlice` (SRV-04): one `at=` slice at file, module or service level.
//!
//! The slice is the live files and the live links at one step, scanned from SQLite (5 ms for 115k
//! rows in phase 0). Positions are the indexer's stored layout (IDX-25, I2) live at that step; the
//! query layer never computes layout. A node with no layout row (an index migrated but not synced
//! again) keeps x = y = 0 and size 1, and the client lays it out itself.
//!
//! Module and service levels roll the file slice up: each file stands for its module (or its
//! service when it sits in no module) or for its service, and the links between two holders
//! become one link whose `weight` counts the file links (SRV-04's count). At module level the
//! services that hold those modules are in the slice too, as their parents. A roll-up reads the
//! generation's in-memory graph (file and link intervals, holders and their positions), not the
//! tables: a few milliseconds once the graph is built, which a server does as soon as it sees a
//! new generation (SRV-04's "hold the intervals in memory per index generation").
//!
//! ponytail: `scope` and `range=` are still to come; the slice is built in memory, not streamed
//! from the cursor: the response cap bounds it.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use cartograph_api::types::{
    ConfidenceLevel, GraphParams, GraphSlice, LinkColumns, LinkKind, Lod, NodeColumns, NodeKind,
};
use cartograph_core::{LinkId, NodeId};
use cartograph_store::rusqlite::{self, OptionalExtension};

use crate::adjacency::{Adjacency, Kind, NONE};
use crate::resolve::{link_id, node_id};
use crate::{At, QueryError, Snapshot, level};

/// Response cap: above it the request fails with `too_large` (SRV-04).
pub const MAX_NODES: usize = 100_000;
pub const MAX_LINKS: usize = 400_000;

/// A checked graph request.
#[derive(Debug, Clone)]
pub struct GraphQuery {
    pub at: At,
    /// File, module or service (SRV-04).
    pub lod: Lod,
    /// None keeps every kind.
    pub kinds: Option<Vec<LinkKind>>,
    pub min_confidence: ConfidenceLevel,
    pub repo: Option<String>,
}

fn later(what: &str, when: &str) -> QueryError {
    QueryError::bad(
        format!("{what} isn't available yet"),
        format!("this index has files and import links only; {what} arrives with {when}"),
        "ask for lod=file, module or service with at=",
    )
}

fn rank(l: ConfidenceLevel) -> u8 {
    match l {
        ConfidenceLevel::Guessed => 0,
        ConfidenceLevel::Likely => 1,
        ConfidenceLevel::Exact => 2,
    }
}

impl GraphQuery {
    /// Checks the HTTP parameters; what phase 1 can't answer is refused by name, never ignored.
    pub fn from_params(p: &GraphParams) -> Result<Self, QueryError> {
        let lod = match p.lod {
            None | Some(Lod::File) => Lod::File,
            Some(l @ (Lod::Module | Lod::Service)) => l,
            Some(Lod::Workspace) => {
                return Err(later("the workspace level of detail", "domains (SRV-04)"));
            }
        };
        if p.scope.is_some() {
            return Err(later("scope", "in-place expand (SRV-04)"));
        }
        if p.range.is_some() {
            return Err(later("range", "phase 3 (SRV-04); ask for one at= per step"));
        }
        if p.owner.is_some() {
            return Err(later("the owner filter", "CODEOWNERS in phase 4"));
        }
        if p.branch.is_some() {
            return Err(later("the branch filter", "branch overlays (IDX-24)"));
        }
        let kinds =
            p.kinds
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .filter(|k| !k.is_empty())
                        .map(|k| {
                            serde_json::from_value(serde_json::Value::String(k.trim().into()))
                                .map_err(|_| {
                                    QueryError::bad(
                                        "kinds names an unknown link kind",
                                        "link kinds are imports, calls, http, event and db",
                                        "for example kinds=imports,http",
                                    )
                                })
                        })
                        .collect::<Result<Vec<LinkKind>, _>>()
                })
                .transpose()?;
        Ok(Self {
            at: At::parse(p.at.as_deref().unwrap_or("now"))?,
            lod,
            kinds,
            // H2: scores under 0.30 are hidden by default, and every level starts at 0.30.
            min_confidence: p.min_confidence.unwrap_or(ConfidenceLevel::Guessed),
            repo: p.repo.as_ref().map(|r| r.0.clone()),
        })
    }
}

/// A live file: its node, its path, the node one level up and its service.
struct LiveFile {
    id: i64,
    path: String,
    parent: Option<i64>,
    /// Read only by the SQL roll-up the tests compare the in-memory one against.
    #[cfg_attr(not(test), allow(dead_code))]
    service: Option<i64>,
    pos: Option<(f32, f32, f32)>,
}

fn empty_nodes() -> NodeColumns {
    NodeColumns {
        id: Vec::new(),
        kind: Vec::new(),
        name: Vec::new(),
        parent: Vec::new(),
        x: Vec::new(),
        y: Vec::new(),
        size: Vec::new(),
        children: Vec::new(),
    }
}

fn empty_links() -> LinkColumns {
    LinkColumns {
        id: Vec::new(),
        from: Vec::new(),
        to: Vec::new(),
        kind: Vec::new(),
        level: Vec::new(),
        score: Vec::new(),
        corrected: Vec::new(),
        weight: Vec::new(),
        added_seq: Vec::new(),
        removed_seq: Vec::new(),
    }
}

/// Appends one node; no layout row means 0, 0 and size 1.
fn push_node(
    nodes: &mut NodeColumns,
    id: i64,
    kind: NodeKind,
    name: String,
    pos: Option<(f32, f32, f32)>,
) {
    let (x, y, r) = pos.unwrap_or((0.0, 0.0, 1.0));
    nodes.id.push(NodeId(node_id(id)));
    nodes.kind.push(kind);
    nodes.name.push(name);
    nodes.parent.push(-1);
    nodes.x.push(x);
    nodes.y.push(y);
    nodes.size.push(r);
    nodes.children.push(0);
}

/// Parsed imports are exact at 1.0; HTTP links carry the score the indexer stored (XS-05). The
/// filters apply per link.
fn keeps_link(q: &GraphQuery, kind: LinkKind, level: ConfidenceLevel) -> bool {
    q.kinds.as_ref().is_none_or(|ks| ks.contains(&kind)) && rank(level) >= rank(q.min_confidence)
}

/// Refuses one more live file past the node cap or the row budget: the same at every level.
fn check_files(snap: &Snapshot<'_>, live: usize) -> Result<(), QueryError> {
    if live >= MAX_NODES {
        return Err(QueryError::TooLarge {
            why: format!("more than {MAX_NODES} files are live at this commit"),
            narrower: NARROWER.into(),
        });
    }
    snap.check_rows(live + 1, NARROWER)
}

/// A roll-up's link index by (holder, holder) pair. 1,202 modules make 185k pairs out of 200k
/// links, so this lookup runs once per live link: a dense n x n table (one load, 5.8 MB at 1,202
/// holders) while it stays under [`DENSE_PAIRS`] cells, a hash map past that.
enum Pairs {
    /// Link index + 1, 0 for none; row-major.
    Dense(Vec<u32>, usize),
    Sparse(HashMap<(u32, u32), u32>),
}

/// 16 MB of `u32`: 2,048 holders.
const DENSE_PAIRS: usize = 1 << 22;

impl Pairs {
    fn new(n: usize) -> Self {
        if n * n <= DENSE_PAIRS {
            // Zeroed memory comes from the allocator untouched: only the rows used are paged in.
            Self::Dense(vec![0; n * n], n)
        } else {
            Self::Sparse(HashMap::new())
        }
    }

    /// The link already made for `(a, b)`, or `None` after recording `next` as it.
    fn get_or_put(&mut self, a: u32, b: u32, next: usize) -> Option<usize> {
        match self {
            Self::Dense(cells, n) => {
                let c = &mut cells[a as usize * *n + b as usize];
                if *c == 0 {
                    *c = next as u32 + 1;
                    None
                } else {
                    Some(*c as usize - 1)
                }
            }
            Self::Sparse(m) => match m.entry((a, b)) {
                Entry::Occupied(j) => Some(*j.get() as usize),
                Entry::Vacant(v) => {
                    v.insert(next as u32);
                    None
                }
            },
        }
    }
}

const NARROWER: &str = "ask for fewer link kinds (kinds=) or one repo (repo=)";

impl Snapshot<'_> {
    /// The slice, from the per-generation cache when the same request was answered already.
    /// Shared with that cache, not copied: a copy of 185k link ids costs what a roll-up does.
    pub fn graph(&self, q: &GraphQuery) -> Result<Arc<GraphSlice>, QueryError> {
        // The row cap is part of the request: a slice built under a larger one may pass a smaller.
        let key = format!("{q:?} max_rows={}", self.limits.max_rows);
        if let Some(s) = self.cached_slice(&key) {
            return Ok(s);
        }
        let s = Arc::new(self.build_graph(q)?);
        self.keep_slice(&key, s.clone());
        Ok(s)
    }

    /// The files live at `seq` (of `repo` when given), by path, with their stored positions.
    fn live_files(
        &self,
        seq: i64,
        repo: Option<&String>,
        paths: bool,
    ) -> Result<Vec<LiveFile>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT n.id, fp.path, n.parent, n.service
             FROM file_path fp JOIN node n ON n.file = fp.file
             JOIN file f ON f.id = fp.file
             WHERE fp.from_seq <= ?1 AND (fp.to_seq IS NULL OR fp.to_seq > ?1)
               AND (?2 IS NULL OR f.repo IN (SELECT id FROM repo WHERE name = ?2))",
        )?;
        // One scan of the live layout beats a per-file index probe: about 3 ms against 40-150 ms
        // on 60k files. Two rows live for one node would be the indexer's bug: the first wins.
        let mut pos: HashMap<i64, (f32, f32, f32)> = HashMap::new();
        let mut lt = self.conn().prepare_cached(
            "SELECT node, x, y, r FROM layout
             WHERE from_seq <= ?1 AND (to_seq IS NULL OR to_seq > ?1)",
        )?;
        let mut lrows = lt.query([seq])?;
        while let Some(r) = lrows.next()? {
            pos.entry(r.get(0)?).or_insert((
                r.get::<_, f64>(1)? as f32,
                r.get::<_, f64>(2)? as f32,
                r.get::<_, f64>(3)? as f32,
            ));
        }
        let mut rows = st.query(rusqlite::params![seq, repo])?;
        let mut out: Vec<LiveFile> = Vec::new();
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            check_files(self, out.len())?;
            out.push(LiveFile {
                id,
                // A roll-up never shows a path: skip the 60k allocations and the sort.
                path: if paths { r.get(1)? } else { String::new() },
                parent: r.get(2)?,
                service: r.get(3)?,
                pos: pos.get(&id).copied(),
            });
        }
        // Sorted here, not by ORDER BY: walking the path index costs ~5x a scan and a sort. Byte
        // order, as SQLite's BINARY collation gives.
        if paths {
            out.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Ok(out)
    }

    /// Calls `each(src, dst, from_seq, to_seq, link, kind, score)` for every live link at `seq`
    /// the filters keep.
    fn live_links(
        &self,
        q: &GraphQuery,
        seq: i64,
        mut each: impl FnMut(i64, i64, i64, Option<i64>, i64, LinkKind, f32) -> Result<(), QueryError>,
    ) -> Result<(), QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT l.id, l.src, l.dst, i.from_seq, i.to_seq, l.kind = 'http', l.score
             FROM interval i JOIN link l ON l.id = i.link
             WHERE i.from_seq <= ?1 AND (i.to_seq IS NULL OR i.to_seq > ?1)",
        )?;
        // Sorted here, not by ORDER BY: SQLite's temp B-tree costs ~3x the scan on 200k links.
        type Live = (i64, i64, i64, i64, Option<i64>, LinkKind, f32);
        let mut live: Vec<Live> = Vec::new();
        let mut rows = st.query([seq])?;
        while let Some(r) = rows.next()? {
            let kind = if r.get::<_, bool>(5)? {
                LinkKind::Http
            } else {
                LinkKind::Imports
            };
            let score = r.get::<_, Option<f64>>(6)?.map_or(1.0, |s| s as f32);
            if keeps_link(q, kind, level(score)) {
                live.push((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    kind,
                    score,
                ));
            }
        }
        live.sort_unstable_by_key(|l| l.0);
        for (id, src, dst, from_seq, to_seq, kind, score) in live {
            each(src, dst, from_seq, to_seq, id, kind, score)?;
        }
        Ok(())
    }

    fn build_graph(&self, q: &GraphQuery) -> Result<GraphSlice, QueryError> {
        let at = self.resolve_at(&q.at)?;
        let seq = at.seq as i64;
        // A repo dropped from the workspace keeps its history, so `at=` before its removal can
        // still name it: any repo row counts, not only the listed ones.
        if let Some(repo) = &q.repo
            && self
                .conn()
                .prepare_cached("SELECT 1 FROM repo WHERE name = ?1")?
                .query_row([repo], |_| Ok(()))
                .optional()?
                .is_none()
        {
            return Err(QueryError::NotFound {
                what: "no repo has that name in this workspace".into(),
                action: "list repos with GET …/repos".into(),
            });
        }
        // repo= keeps that repo's files; links with an end outside them are dropped below.
        let (nodes, links) = match q.lod {
            Lod::Module | Lod::Service => self.rolled_up(&*self.adjacency()?, q, at.seq as u32)?,
            _ => {
                let files = self.live_files(seq, q.repo.as_ref(), true)?;
                self.file_level(q, seq, files)?
            }
        };
        Ok(GraphSlice {
            lod: q.lod,
            scope: None,
            at: Some(at),
            range: None,
            nodes,
            links,
        })
    }

    fn file_level(
        &self,
        q: &GraphQuery,
        seq: i64,
        files: Vec<LiveFile>,
    ) -> Result<(NodeColumns, LinkColumns), QueryError> {
        let mut nodes = empty_nodes();
        let index: HashMap<i64, u32> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i as u32))
            .collect();
        let mut parents = Vec::with_capacity(files.len());
        for f in files {
            parents.push(f.parent);
            // The repo-relative path: unique, and what a file is known by.
            push_node(&mut nodes, f.id, NodeKind::File, f.path, f.pos);
        }
        // A parent is in the slice only when it is a file's (never, today): -1 otherwise.
        for (i, p) in parents.into_iter().enumerate() {
            if let Some(&j) = p.and_then(|p| index.get(&p)) {
                nodes.parent[i] = j as i32;
            }
        }

        let mut links = empty_links();
        let n = nodes.id.len();
        self.live_links(q, seq, |src, dst, from_seq, to_seq, id, kind, score| {
            let (Some(&from), Some(&to)) = (index.get(&src), index.get(&dst)) else {
                // With repo=, a link into another repo has its other end outside the slice.
                // Otherwise the indexer only links files it indexed; a dangling end is its bug,
                // not ours to paper over silently.
                if q.repo.is_none() {
                    tracing::warn!(
                        src,
                        dst,
                        seq,
                        "a live link has an end that isn't a live file"
                    );
                }
                return Ok(());
            };
            if links.id.len() >= MAX_LINKS {
                return Err(QueryError::TooLarge {
                    why: format!("more than {MAX_LINKS} links are live at this commit"),
                    narrower: NARROWER.into(),
                });
            }
            self.check_rows(n + links.id.len() + 1, NARROWER)?;
            links.id.push(LinkId(link_id(id)));
            links.from.push(from);
            links.to.push(to);
            links.kind.push(kind);
            links.level.push(level(score));
            links.score.push(score);
            links.corrected.push(false);
            links.weight.push(1);
            links.added_seq.push(from_seq as u64);
            links.removed_seq.push(to_seq.unwrap_or(-1));
            Ok(())
        })?;
        Ok((nodes, links))
    }

    /// The roll-up read from the tables, as it was before the in-memory one: the reference
    /// [`Self::rolled_up`] is tested against. Takes a checked repo name.
    #[cfg(test)]
    pub(crate) fn rolled_up_sql(&self, q: &GraphQuery) -> Result<GraphSlice, QueryError> {
        let at = self.resolve_at(&q.at)?;
        let seq = at.seq as i64;
        let files = self.live_files(seq, q.repo.as_ref(), false)?;
        let (nodes, links) = self.rolled_up_from(q, seq, &files)?;
        Ok(GraphSlice {
            lod: q.lod,
            scope: None,
            at: Some(at),
            range: None,
            nodes,
            links,
        })
    }

    #[cfg(test)]
    fn rolled_up_from(
        &self,
        q: &GraphQuery,
        seq: i64,
        files: &[LiveFile],
    ) -> Result<(NodeColumns, LinkColumns), QueryError> {
        // Services, libraries and modules: kind, name, parent and position at `seq`.
        struct Holder {
            kind: String,
            name: String,
            parent: Option<i64>,
            pos: Option<(f32, f32, f32)>,
        }
        let mut holders: HashMap<i64, Holder> = HashMap::new();
        let mut st = self.conn().prepare_cached(
            "SELECT n.id, n.kind, coalesce(n.name, n.key, ''), n.parent, l.x, l.y, l.r
             FROM node n LEFT JOIN layout l ON l.node = n.id AND l.from_seq <= ?1
               AND (l.to_seq IS NULL OR l.to_seq > ?1)
             WHERE n.kind IN ('service', 'library', 'module')",
        )?;
        let mut rows = st.query([seq])?;
        while let Some(r) = rows.next()? {
            let pos = match (
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<f64>>(6)?,
            ) {
                (Some(x), Some(y), Some(rr)) => Some((x as f32, y as f32, rr as f32)),
                _ => None,
            };
            holders.entry(r.get(0)?).or_insert(Holder {
                kind: r.get(1)?,
                name: r.get(2)?,
                parent: r.get(3)?,
                pos,
            });
        }
        drop(rows);
        let is_module = |id: i64| holders.get(&id).is_some_and(|h| h.kind == "module");
        // Each file's holder at this level: its module, or its service when it is in none.
        let module_level = q.lod == Lod::Module;
        let holder_of = |f: &LiveFile| -> Option<i64> {
            let h = if module_level {
                f.parent.or(f.service)
            } else {
                f.service
            };
            h.filter(|h| holders.contains_key(h))
        };
        let of: HashMap<i64, i64> = files
            .iter()
            .filter_map(|f| Some((f.id, holder_of(f)?)))
            .collect();
        // The nodes: the holders some live file stands for, and at module level their services.
        let mut members: HashMap<i64, u32> = HashMap::new();
        let mut modules_of: HashMap<i64, std::collections::BTreeSet<i64>> = HashMap::new();
        for f in files {
            if let Some(&h) = of.get(&f.id) {
                *members.entry(h).or_default() += 1;
            }
            if let (Some(p), Some(s)) = (f.parent, f.service)
                && is_module(p)
                && holders.contains_key(&s)
            {
                modules_of.entry(s).or_default().insert(p);
            }
        }
        let mut ids: Vec<i64> = members.keys().copied().collect();
        if module_level {
            let services: Vec<i64> = ids
                .iter()
                .filter_map(|h| is_module(*h).then(|| holders[h].parent).flatten())
                .filter(|s| holders.contains_key(s))
                .collect();
            ids.extend(services);
        }
        // Services first, then modules, each by name: the same order every time.
        ids.sort_by(|a, b| {
            (is_module(*a), &holders[a].name, a).cmp(&(is_module(*b), &holders[b].name, b))
        });
        ids.dedup();
        let mut nodes = empty_nodes();
        let index: HashMap<i64, u32> = ids
            .iter()
            .enumerate()
            .map(|(i, h)| (*h, i as u32))
            .collect();
        for &h in &ids {
            let x = &holders[&h];
            let kind = if x.kind == "module" {
                NodeKind::Module
            } else {
                // A library is a service that deploys nothing (C6); the wire has one kind for both.
                NodeKind::Service
            };
            push_node(&mut nodes, h, kind, x.name.clone(), x.pos);
            let i = nodes.id.len() - 1;
            if let Some(&p) = x.parent.as_ref().and_then(|p| index.get(p)) {
                nodes.parent[i] = p as i32;
            }
            // One level down: a service's modules with a live file, a module's live files.
            nodes.children[i] = if kind == NodeKind::Module {
                members.get(&h).copied().unwrap_or(0)
            } else {
                modules_of.get(&h).map_or(0, |m| m.len() as u32)
            };
        }
        self.check_rows(nodes.id.len(), NARROWER)?;

        // Links: one per (holder, holder) pair, first seen in link id order.
        let mut links = empty_links();
        let mut pair: HashMap<(u32, u32), usize> = HashMap::new();
        let n = nodes.id.len();
        // File id to holder index, dense: two hash probes per end were most of the 200k-link scan.
        let top = files.iter().map(|f| f.id).max().unwrap_or(0).max(0) as usize;
        let mut slot = vec![u32::MAX; top + 1];
        for (f, h) in &of {
            if let Some(&i) = index.get(h) {
                slot[*f as usize] = i;
            }
        }
        let slot_of = |id: i64| {
            usize::try_from(id)
                .ok()
                .and_then(|i| slot.get(i))
                .filter(|i| **i != u32::MAX)
        };
        self.live_links(q, seq, |src, dst, from_seq, _, _, kind, score| {
            let (Some(a), Some(b)) = (slot_of(src), slot_of(dst)) else {
                return Ok(());
            };
            if a == b {
                return Ok(());
            }
            if let Some(&j) = pair.get(&(*a, *b)) {
                links.weight[j] += 1;
                links.added_seq[j] = links.added_seq[j].min(from_seq as u64);
                return Ok(());
            }
            self.check_rows(n + links.id.len() + 1, NARROWER)?;
            pair.insert((*a, *b), links.id.len());
            links.id.push(LinkId(format!(
                "{}>{}",
                nodes.id[*a as usize].0, nodes.id[*b as usize].0
            )));
            links.from.push(*a);
            links.to.push(*b);
            links.kind.push(kind);
            links.level.push(level(score));
            links.score.push(score);
            links.corrected.push(false);
            links.weight.push(1);
            links.added_seq.push(from_seq as u64);
            links.removed_seq.push(-1);
            Ok(())
        })?;
        Ok((nodes, links))
    }

    /// Module or service level: each file stands for its holder at that level, and the links
    /// between two holders become one, `weight` counting the file links. Read from the
    /// generation's in-memory graph; the same answer the tables give (`rolled_up_sql`).
    fn rolled_up(
        &self,
        g: &Adjacency,
        q: &GraphQuery,
        seq: u32,
    ) -> Result<(NodeColumns, LinkColumns), QueryError> {
        // repo= keeps the files of every repo row with that name.
        let repos: Option<Vec<i64>> = match &q.repo {
            Some(name) => Some(
                self.conn()
                    .prepare_cached("SELECT id FROM repo WHERE name = ?1")?
                    .query_map([name], |r| r.get(0))?
                    .collect::<Result<_, _>>()?,
            ),
            None => None,
        };
        // Every node that isn't a file is a holder (`g.holders` has them all): one load, no hash.
        let holder = |h: u32| h != NONE && g.kind[h as usize] != Kind::File;
        let is_module = |h: u32| holder(h) && g.kind[h as usize] == Kind::Module;
        let module_level = q.lod == Lod::Module;
        // Each live file's holder at this level: its module, or its service when it is in none.
        // Dense by node index: hashing 60k files was most of this loop.
        let mut of = vec![NONE; g.row.len()];
        let mut members = vec![0u32; g.row.len()];
        let mut ids: Vec<u32> = Vec::new();
        // A service's modules with a live file: each module is counted under the first service
        // one of its files names, and under any other (never, today) through `extra`.
        let mut module_service = vec![NONE; g.row.len()];
        let mut modules_of = vec![0u32; g.row.len()];
        let mut extra: std::collections::HashSet<(u32, u32)> = Default::default();
        let mut live = 0;
        for f in &g.files {
            if !(f.from <= seq && seq < f.to)
                || repos.as_ref().is_some_and(|r| !r.contains(&f.repo))
            {
                continue;
            }
            check_files(self, live)?;
            live += 1;
            let n = f.node as usize;
            let (parent, service) = (g.parent[n], g.service[n]);
            let h = if module_level && parent != NONE {
                parent
            } else {
                service
            };
            if holder(h) {
                of[n] = h;
                if members[h as usize] == 0 {
                    ids.push(h);
                }
                members[h as usize] += 1;
            }
            if is_module(parent) && holder(service) {
                let first = &mut module_service[parent as usize];
                if *first == NONE {
                    *first = service;
                    modules_of[service as usize] += 1;
                } else if *first != service && extra.insert((service, parent)) {
                    modules_of[service as usize] += 1;
                }
            }
        }
        // The nodes: the holders some live file stands for, and at module level their services.
        if module_level {
            let services: Vec<u32> = ids
                .iter()
                .filter(|h| is_module(**h))
                .map(|h| g.parent[*h as usize])
                .filter(|s| holder(*s))
                .collect();
            ids.extend(services);
        }
        // Services first, then modules, each by name: the same order every time.
        let key = |h: &u32| (is_module(*h), &g.holders[h].name, g.row[*h as usize]);
        ids.sort_by(|a, b| key(a).cmp(&key(b)));
        ids.dedup();
        let mut nodes = empty_nodes();
        let mut index = vec![NONE; g.row.len()];
        for (i, &h) in ids.iter().enumerate() {
            index[h as usize] = i as u32;
        }
        for (i, &h) in ids.iter().enumerate() {
            let x = &g.holders[&h];
            let kind = if is_module(h) {
                NodeKind::Module
            } else {
                // A library is a service that deploys nothing (C6); the wire has one kind for both.
                NodeKind::Service
            };
            push_node(
                &mut nodes,
                g.row[h as usize],
                kind,
                x.name.clone(),
                x.pos(seq),
            );
            let p = g.parent[h as usize];
            if p != NONE && index[p as usize] != NONE {
                nodes.parent[i] = index[p as usize] as i32;
            }
            // One level down: a service's modules with a live file, a module's live files.
            nodes.children[i] = if kind == NodeKind::Module {
                members[h as usize]
            } else {
                modules_of[h as usize]
            };
        }
        self.check_rows(nodes.id.len(), NARROWER)?;

        // Links: one per (holder, holder) pair, first seen in link id order.
        let mut links = empty_links();
        // Node to slice index, live files only: a link with any other end is dropped.
        let slot: Vec<u32> = of
            .iter()
            .map(|&h| if h == NONE { NONE } else { index[h as usize] })
            .collect();
        // The columns are sized for every edge or every pair, whichever is fewer, so they never
        // grow: growing them cost a fifth of the module level.
        let n = nodes.id.len();
        let most = g.edges.len().min(n * n);
        let mut pair = Pairs::new(n);
        links.id.reserve(most);
        links.from.reserve(most);
        links.to.reserve(most);
        links.kind.reserve(most);
        links.level.reserve(most);
        links.score.reserve(most);
        links.corrected.reserve(most);
        links.weight.reserve(most);
        links.added_seq.reserve(most);
        links.removed_seq.reserve(most);
        for e in &g.edges {
            if !e.live(seq) || !keeps_link(q, e.kind, level(e.score)) {
                continue;
            }
            let (a, b) = (slot[e.src as usize], slot[e.dst as usize]);
            if a == NONE || b == NONE || a == b {
                continue;
            }
            if let Some(j) = pair.get_or_put(a, b, links.id.len()) {
                links.weight[j] += 1;
                links.added_seq[j] = links.added_seq[j].min(u64::from(e.from));
                continue;
            }
            self.check_rows(n + links.id.len() + 1, NARROWER)?;
            let (from, to) = (&nodes.id[a as usize].0, &nodes.id[b as usize].0);
            let mut id = String::with_capacity(from.len() + 1 + to.len());
            id.push_str(from);
            id.push('>');
            id.push_str(to);
            links.id.push(LinkId(id));
            links.from.push(a);
            links.to.push(b);
            links.kind.push(e.kind);
            links.level.push(level(e.score));
            links.score.push(e.score);
            links.corrected.push(false);
            links.weight.push(1);
            links.added_seq.push(u64::from(e.from));
            links.removed_seq.push(-1);
        }
        Ok((nodes, links))
    }
}

#[cfg(test)]
mod pairs_tests {
    use super::Pairs;

    /// Past 2,048 holders the table is a hash map; both give the same first-seen indexes.
    #[test]
    fn dense_and_sparse_pairs_agree() {
        let mut dense = Pairs::new(3);
        let mut sparse = Pairs::Sparse(Default::default());
        assert!(matches!(dense, Pairs::Dense(..)));
        assert!(matches!(Pairs::new(2049), Pairs::Sparse(_)));
        for (i, (a, b)) in [(0, 1), (1, 0), (0, 1), (2, 2), (1, 0)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(dense.get_or_put(a, b, i), sparse.get_or_put(a, b, i));
        }
        assert_eq!(dense.get_or_put(0, 1, 9), Some(0));
        assert_eq!(sparse.get_or_put(1, 0, 9), Some(1));
    }
}
