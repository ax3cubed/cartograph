//! The in-memory graph of one generation (SRV-11): every node's kind, parent and service, every
//! link interval as an edge, and CSR offsets in both directions. Built lazily on first use from one
//! snapshot, held in an `Arc` on the [`Store`](crate::Store) and replaced when the generation (or
//! the file) changes, so HTTP, MCP and Ask share it.
//!
//! Metrics, neighbours, ranking, changes, density and the traversals all run over it instead of a
//! query per request: 200k links is a few MB and a scan of them is about a millisecond. The module
//! and service slices roll up from it too (SRV-04): it holds each file's path intervals and repo,
//! and each holder's name and stored positions, so a roll-up reads no table.

use std::collections::HashMap;

use cartograph_api::types::LinkKind;

use crate::{QueryError, Snapshot};

/// No parent, no service, or an interval still open.
pub(crate) const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Module,
    Service,
    Library,
}

/// One interval of one link: the link is live at steps `from <= s < to`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Edge {
    pub link: i64,
    pub src: u32,
    pub dst: u32,
    pub kind: LinkKind,
    pub from: u32,
    /// [`NONE`] while open.
    pub to: u32,
    /// Parsed imports are 1.0 until other layers write scores (XS items).
    pub score: f32,
}

impl Edge {
    pub fn live(&self, seq: u32) -> bool {
        self.from <= seq && seq < self.to
    }
}

/// One path interval of one file node: the file is live at steps `from <= s < to` (IDX-18).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileSpan {
    pub node: u32,
    /// The file's repo row id.
    pub repo: i64,
    pub from: u32,
    /// [`NONE`] while open.
    pub to: u32,
}

/// A service, library or module: what a roll-up shows.
#[derive(Debug, Clone)]
pub(crate) struct Holder {
    /// `coalesce(name, key, '')`.
    pub name: String,
    /// Stored positions (IDX-25) as `(from, to, x, y, r)`, by `from`: the first live one is read.
    pub layout: Vec<(u32, u32, f32, f32, f32)>,
}

impl Holder {
    pub fn pos(&self, seq: u32) -> Option<(f32, f32, f32)> {
        self.layout
            .iter()
            .find(|l| l.0 <= seq && seq < l.1)
            .map(|l| (l.2, l.3, l.4))
    }
}

pub(crate) struct Adjacency {
    pub key: (u64, u64),
    /// Dense node index -> node row id.
    pub row: Vec<i64>,
    pub index: HashMap<i64, u32>,
    pub kind: Vec<Kind>,
    pub parent: Vec<u32>,
    pub service: Vec<u32>,
    pub edges: Vec<Edge>,
    out_off: Vec<u32>,
    out_e: Vec<u32>,
    in_off: Vec<u32>,
    in_e: Vec<u32>,
    /// Corrected commit time by seq, and each step's repo row id.
    pub time: Vec<i64>,
    pub repo: Vec<i64>,
    /// Every file node's path intervals, in no particular order.
    pub files: Vec<FileSpan>,
    /// Services, libraries and modules by node index.
    pub holders: HashMap<u32, Holder>,
}

/// What a scope covers: the whole workspace, or a node and what lies below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    All,
    Node(u32),
}

impl Adjacency {
    pub fn build(snap: &Snapshot<'_>, key: (u64, u64)) -> Result<Self, QueryError> {
        let c = snap.conn();
        let mut row = Vec::new();
        let mut kind = Vec::new();
        let mut parent_row = Vec::new();
        let mut service_row = Vec::new();
        let mut holders = HashMap::new();
        let mut st = c.prepare_cached(
            "SELECT id, kind, parent, service,
               CASE WHEN kind IN ('service', 'library', 'module')
                 THEN coalesce(name, key, '') END
             FROM node ORDER BY id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            row.push(r.get::<_, i64>(0)?);
            kind.push(match r.get_ref(1)?.as_str().unwrap_or("file") {
                "module" => Kind::Module,
                "service" => Kind::Service,
                "library" => Kind::Library,
                _ => Kind::File,
            });
            parent_row.push(r.get::<_, Option<i64>>(2)?);
            service_row.push(r.get::<_, Option<i64>>(3)?);
            if let Some(name) = r.get::<_, Option<String>>(4)? {
                let layout = Vec::new();
                holders.insert(row.len() as u32 - 1, Holder { name, layout });
            }
        }
        drop(rows);
        let index: HashMap<i64, u32> = row
            .iter()
            .enumerate()
            .map(|(i, &r)| (r, i as u32))
            .collect();
        // Row id to dense index without hashing: ids are near-contiguous (`row` is sorted), so a
        // vector over them is a few hundred KB and every per-link lookup below is one load.
        let top = row.last().copied().unwrap_or(0).max(0) as usize;
        let mut slot = vec![NONE; top + 1];
        for (i, &r) in row.iter().enumerate() {
            if r >= 0 {
                slot[r as usize] = i as u32;
            }
        }
        let at = |r: i64| -> Option<u32> {
            usize::try_from(r)
                .ok()
                .and_then(|i| slot.get(i))
                .copied()
                .filter(|i| *i != NONE)
        };
        let dense = |r: Option<i64>| r.and_then(at).unwrap_or(NONE);
        let parent = parent_row.into_iter().map(dense).collect();
        let service = service_row.into_iter().map(dense).collect();

        let mut edges = Vec::new();
        let mut st = c.prepare_cached(
            "SELECT l.id, l.kind, l.src, l.dst, i.from_seq, i.to_seq, l.score
             FROM interval i JOIN link l ON l.id = i.link",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let (Some(src), Some(dst)) = (at(r.get::<_, i64>(2)?), at(r.get::<_, i64>(3)?)) else {
                continue;
            };
            edges.push(Edge {
                link: r.get(0)?,
                src,
                dst,
                // `package` links are imports of a workspace package; the wire has one kind for both.
                kind: if r.get_ref(1)?.as_str() == Ok("http") {
                    LinkKind::Http
                } else {
                    LinkKind::Imports
                },
                from: r.get::<_, i64>(4)? as u32,
                to: r.get::<_, Option<i64>>(5)?.map_or(NONE, |t| t as u32),
                // Only HTTP links carry a score (XS-05); imports are parsed, so 1.0.
                score: r.get::<_, Option<f64>>(6)?.map_or(1.0, |s| s as f32),
            });
        }
        drop(rows);
        // Link id, then interval start: sorted here, not by ORDER BY, whose temp B-tree costs about
        // three times the scan on 200k intervals.
        edges.sort_unstable_by_key(|e| (e.link, e.from));
        snap.check_rows(
            row.len() + edges.len(),
            "the index is over the response cap",
        )?;

        let n = row.len();
        let csr = |end: fn(&Edge) -> u32| {
            let mut off = vec![0u32; n + 1];
            for e in &edges {
                off[end(e) as usize + 1] += 1;
            }
            for i in 0..n {
                off[i + 1] += off[i];
            }
            let mut fill = off.clone();
            let mut out = vec![0u32; edges.len()];
            for (i, e) in edges.iter().enumerate() {
                let at = &mut fill[end(e) as usize];
                out[*at as usize] = i as u32;
                *at += 1;
            }
            (off, out)
        };
        let (out_off, out_e) = csr(|e| e.src);
        let (in_off, in_e) = csr(|e| e.dst);

        // A holder's positions in the layout index's order, as the SQL roll-up's join read them.
        let mut st = c.prepare_cached(
            "SELECT l.node, l.x, l.y, l.r, l.from_seq, l.to_seq FROM layout l
             JOIN node n ON n.id = l.node
             WHERE n.kind IN ('service', 'library', 'module')
             ORDER BY l.node, l.from_seq, l.rowid",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some(h) = at(r.get::<_, i64>(0)?).and_then(|n| holders.get_mut(&n)) else {
                continue;
            };
            h.layout.push((
                r.get::<_, i64>(4)? as u32,
                r.get::<_, Option<i64>>(5)?.map_or(NONE, |t| t as u32),
                r.get::<_, f64>(1)? as f32,
                r.get::<_, f64>(2)? as f32,
                r.get::<_, f64>(3)? as f32,
            ));
        }
        drop(rows);

        // Every file node's path intervals, with its repo: the files a slice at `s` holds.
        let mut files = Vec::new();
        let mut st = c.prepare_cached(
            "SELECT n.id, f.repo, fp.from_seq, fp.to_seq
             FROM file_path fp JOIN node n ON n.file = fp.file JOIN file f ON f.id = fp.file",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some(node) = at(r.get::<_, i64>(0)?) else {
                continue;
            };
            files.push(FileSpan {
                node,
                repo: r.get(1)?,
                from: r.get::<_, i64>(2)? as u32,
                to: r.get::<_, Option<i64>>(3)?.map_or(NONE, |t| t as u32),
            });
        }
        drop(rows);

        let mut time = Vec::new();
        let mut repo = Vec::new();
        let mut st = c.prepare_cached("SELECT corrected_time, repo FROM commits ORDER BY seq")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            time.push(r.get(0)?);
            repo.push(r.get(1)?);
        }
        Ok(Self {
            key,
            row,
            index,
            kind,
            parent,
            service,
            edges,
            out_off,
            out_e,
            in_off,
            in_e,
            time,
            repo,
            files,
            holders,
        })
    }

    /// Edges whose source is `n` (what `n` depends on).
    pub fn out(&self, n: u32) -> impl Iterator<Item = &Edge> {
        let (a, b) = (self.out_off[n as usize], self.out_off[n as usize + 1]);
        self.out_e[a as usize..b as usize]
            .iter()
            .map(|&i| &self.edges[i as usize])
    }

    /// Edges whose target is `n` (what depends on `n`).
    pub fn inbound(&self, n: u32) -> impl Iterator<Item = &Edge> {
        let (a, b) = (self.in_off[n as usize], self.in_off[n as usize + 1]);
        self.in_e[a as usize..b as usize]
            .iter()
            .map(|&i| &self.edges[i as usize])
    }

    /// Node `n` is the scope or below it. `parent` and `service` make this one comparison each:
    /// modules sit directly under a service and files directly under a module or service (IDX-20),
    /// so no walk is needed (SRV-08).
    pub fn within(&self, n: u32, scope: Scope) -> bool {
        match scope {
            Scope::All => true,
            Scope::Node(s) => {
                n == s || self.parent[n as usize] == s || self.service[n as usize] == s
            }
        }
    }

    /// Every node in the scope, the scope first.
    pub fn members(&self, scope: Scope) -> Vec<u32> {
        match scope {
            Scope::All => (0..self.row.len() as u32).collect(),
            Scope::Node(s) => std::iter::once(s)
                .chain((0..self.row.len() as u32).filter(|&n| n != s && self.within(n, scope)))
                .collect(),
        }
    }

    /// The last step at or before time `t`, if any.
    pub fn seq_at(&self, t: i64) -> Option<u32> {
        self.time
            .partition_point(|&x| x <= t)
            .checked_sub(1)
            .map(|s| s as u32)
    }
}
