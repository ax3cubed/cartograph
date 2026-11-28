//! The rows a run writes, shared by a build (IDX-07) and a sync (IDX-14): commits, file lineages
//! across renames (IDX-18), link intervals between lineages, notes and unresolved imports (IDX-22).
//!
//! The replay works on paths; rows work on lineages. A step's links that closed are mapped to nodes
//! with the paths as they were before the step, the ones that opened with the paths after it, and a
//! link between the same two nodes that closed and opened in one step stays open: that is how an
//! importer updated in the commit that moves its target keeps its link (IDX-18 done-when).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cartograph_facts::Replay;
use cartograph_facts::replay::{Change, Link, LinkKind};
use cartograph_git::{CommitStep, ObjectId};
use cartograph_parse::BlobFacts;
use cartograph_store::rusqlite::{self, Connection, params};

use crate::extract::extractor_of;

/// (kind, source node, target node).
pub(crate) type NodeLink = (&'static str, i64, i64);

/// Unresolved rows kept per index (IDX-22, proposed); past it, a counter in `meta`.
pub const UNRESOLVED_CAP: i64 = 1_000_000;
/// Specifiers are cut to this many bytes at write (IDX-22 security).
pub const SPEC_BYTES: usize = 512;

pub(crate) struct Rows<'c> {
    conn: &'c Connection,
    /// Path -> the lineage that last held it: (file id, node id).
    at_path: HashMap<String, (i64, i64)>,
    /// A live lineage's path and open blob.
    alive: HashMap<i64, (String, Option<ObjectId>)>,
    links: HashMap<NodeLink, i64>,
    /// Open links and the step each opened at.
    open: HashMap<NodeLink, i64>,
    /// Links that opened at or before this step have their open row written.
    written: i64,
    /// Repo id -> its last step written: (seq, sha).
    last: HashMap<i64, (i64, ObjectId)>,
    /// Structure nodes by key (a Go import's package folder, IDX-23).
    keyed: HashMap<String, i64>,
    unresolved: i64,
    /// Lineages that got a new path this run (search rows to refresh, AI-15).
    pub(crate) moved: HashSet<i64>,
}

fn oid(bytes: &[u8]) -> Option<ObjectId> {
    ObjectId::try_from(bytes).ok()
}

impl<'c> Rows<'c> {
    /// The lineages, open links and counters of an index truncated to `floor`.
    pub(crate) fn load(conn: &'c Connection, floor: i64) -> rusqlite::Result<Self> {
        let mut at_path = HashMap::new();
        let mut alive = HashMap::new();
        let mut st = conn.prepare(
            "SELECT fp.file, n.id, fp.path, fp.to_seq IS NULL FROM file_path fp
             JOIN node n ON n.file = fp.file AND n.kind = 'file' ORDER BY fp.from_seq, fp.rowid",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let (file, node, path): (i64, i64, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
            if r.get::<_, bool>(3)? {
                alive.insert(file, (path.clone(), None));
            }
            at_path.insert(path, (file, node));
        }
        let mut st = conn.prepare("SELECT file, blob FROM file_version WHERE to_seq IS NULL")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            if let Some(a) = alive.get_mut(&r.get::<_, i64>(0)?) {
                a.1 = oid(r.get_ref(1)?.as_blob()?);
            }
        }
        let mut links = HashMap::new();
        let mut open = HashMap::new();
        let mut st = conn.prepare(
            "SELECT l.kind, l.src, l.dst, l.id, i.from_seq FROM interval i
             JOIN link l ON l.id = i.link WHERE i.to_seq IS NULL AND l.kind NOT IN ('http', 'event')",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let kind = match r.get_ref(0)?.as_str()? {
                "package" => LinkKind::Package.as_str(),
                _ => LinkKind::Import.as_str(),
            };
            let nl = (kind, r.get(1)?, r.get(2)?);
            links.insert(nl, r.get(3)?);
            open.insert(nl, r.get(4)?);
        }
        let keyed = conn
            .prepare("SELECT key, id FROM node WHERE key IS NOT NULL")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let unresolved = conn.query_row("SELECT count(*) FROM unresolved", [], |r| r.get(0))?;
        Ok(Self {
            conn,
            at_path,
            alive,
            links,
            open,
            written: floor,
            last: HashMap::new(),
            keyed,
            unresolved,
            moved: HashSet::new(),
        })
    }

    pub(crate) fn commit(&mut self, seq: i64, repo: i64, s: &CommitStep) -> rusqlite::Result<()> {
        self.last.insert(repo, (seq, s.sha));
        self.commit_only(seq, repo, s)
    }

    /// The commit row alone, without making it the repository's indexed commit.
    pub(crate) fn commit_only(
        &mut self,
        seq: i64,
        repo: i64,
        s: &CommitStep,
    ) -> rusqlite::Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO commits (seq, repo, sha, time, corrected_time, author, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                seq,
                repo,
                s.sha.as_bytes(),
                s.time,
                s.corrected_time,
                s.author,
                // A commit message can carry a pasted key (XS-10).
                cartograph_core::secrets::mask(&s.summary)
            ])?;
        if s.rename_limit {
            self.conn
                .prepare_cached("INSERT INTO note (seq, kind, detail) VALUES (?1, ?2, ?3)")?
                .execute(params![
                    seq,
                    "rename-limit",
                    format!(
                        "more than {} rename candidates: only exact renames were paired",
                        cartograph_git::RENAME_LIMIT
                    )
                ])?;
        }
        Ok(())
    }

    /// Replays one step and writes its rows: the replay first, then the links that closed (mapped
    /// with the paths before the step), the file changes and renames, then the links that opened
    /// (mapped with the paths after it). `path` namespaces a repository path; `changes` are the
    /// step's changes already namespaced, with their facts.
    pub(crate) fn step(
        &mut self,
        seq: i64,
        repo: i64,
        step: &CommitStep,
        path: &dyn Fn(&str) -> String,
        changes: &[Change],
        replay: &mut Replay,
    ) -> rusqlite::Result<()> {
        replay.apply(changes, seq as u64);
        let closed: Vec<NodeLink> = std::mem::take(&mut replay.closed)
            .iter()
            .filter_map(|i| self.node_link(&i.link))
            .collect();
        let gone: Vec<(i64, String)> = std::mem::take(&mut replay.unresolved_closed)
            .into_iter()
            .filter_map(|(from, spec, _)| Some((self.file_at(&from)?, spec)))
            .collect();
        let renamed: HashSet<&str> = step
            .renames
            .iter()
            .flat_map(|r| [r.from.as_str(), r.to.as_str()])
            .collect();
        for r in &step.renames {
            let new = step
                .changes
                .iter()
                .find(|c| c.path == r.to)
                .and_then(|c| c.new);
            self.rename(repo, seq, &path(&r.from), &path(&r.to), new, r)?;
        }
        for c in &step.changes {
            if !renamed.contains(c.path.as_str()) {
                self.file_change(repo, seq, &path(&c.path), c.new)?;
            }
        }
        let opened: Vec<NodeLink> = std::mem::take(&mut replay.opened)
            .iter()
            .filter_map(|(l, _)| self.node_link(l))
            .collect();
        let came: Vec<(i64, String, u32)> = std::mem::take(&mut replay.unresolved_opened)
            .into_iter()
            .filter_map(|(from, spec, line, _)| Some((self.file_at(&from)?, spec, line)))
            .collect();
        let (c, o): (HashSet<_>, HashSet<_>) =
            (closed.into_iter().collect(), opened.into_iter().collect());
        let mut shut: Vec<_> = c.difference(&o).copied().collect();
        shut.sort_unstable();
        for nl in shut {
            self.close(nl, seq)?;
        }
        let mut new: Vec<_> = o.difference(&c).copied().collect();
        new.sort_unstable();
        for nl in new {
            self.open.insert(nl, seq);
        }
        let came_keys: HashSet<(i64, &str)> =
            came.iter().map(|(f, s, _)| (*f, s.as_str())).collect();
        let gone_keys: HashSet<(i64, &str)> = gone.iter().map(|(f, s)| (*f, s.as_str())).collect();
        for (file, spec) in &gone {
            if !came_keys.contains(&(*file, spec.as_str())) {
                self.conn
                    .prepare_cached(
                        "UPDATE unresolved SET to_seq = ?3
                         WHERE file = ?1 AND spec = ?2 AND to_seq IS NULL AND fact_kind IS NULL",
                    )?
                    .execute(params![file, cut(spec), seq])?;
            }
        }
        for (file, spec, line) in &came {
            if gone_keys.contains(&(*file, spec.as_str())) {
                continue;
            }
            if self.unresolved >= UNRESOLVED_CAP {
                self.conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('unresolved_dropped', 1)
                     ON CONFLICT (key) DO UPDATE SET value = value + 1",
                    [],
                )?;
                continue;
            }
            self.unresolved += 1;
            self.conn
                .prepare_cached(
                    "INSERT INTO unresolved (file, spec, line, stage, reason, from_seq)
                     VALUES (?1, ?2, ?3, 'resolve', 'not-found', ?4)",
                )?
                .execute(params![file, cut(spec), line, seq])?;
        }
        Ok(())
    }

    /// The live lineage at `path`.
    fn file_at(&self, path: &str) -> Option<i64> {
        let (file, _) = self.at_path.get(path)?;
        self.alive
            .get(file)
            .is_some_and(|(p, _)| p == path)
            .then_some(*file)
    }

    /// The node at a link end: a live file's node, or a package folder's (`dir/`, Go, IDX-23).
    fn node_at(&mut self, path: &str) -> Option<i64> {
        if let Some(dir) = path.strip_suffix('/') {
            return self.module_node(dir).ok();
        }
        let (file, node) = *self.at_path.get(path)?;
        self.alive
            .get(&file)
            .is_some_and(|(p, _)| p == path)
            .then_some(node)
    }

    fn node_link(&mut self, l: &Link) -> Option<NodeLink> {
        let (Some(src), Some(dst)) = (self.node_at(&l.1), self.node_at(&l.2)) else {
            tracing::warn!(from = %l.1, to = %l.2, "a link end isn't a live file");
            return None;
        };
        Some((l.0.as_str(), src, dst))
    }

    /// The module node of a folder, made when first needed; the structure pass (IDX-20) fills in
    /// the rest of it.
    pub(crate) fn module_node(&mut self, dir: &str) -> rusqlite::Result<i64> {
        let key = format!("module:{dir}");
        if let Some(id) = self.keyed.get(&key) {
            return Ok(*id);
        }
        self.conn
            .prepare_cached("INSERT INTO node (kind, key, name) VALUES ('module', ?1, ?2)")?
            .execute(params![key, dir])?;
        let id = self.conn.last_insert_rowid();
        self.keyed.insert(key, id);
        Ok(id)
    }

    /// A move: the lineage at `from` continues at `to` (IDX-18).
    fn rename(
        &mut self,
        repo: i64,
        seq: i64,
        from: &str,
        to: &str,
        new: Option<ObjectId>,
        r: &cartograph_git::Rename,
    ) -> rusqlite::Result<()> {
        let Some(file) = self.file_at(from) else {
            // The source isn't indexed (a scope or exclude changed): an addition.
            return self.file_change(repo, seq, to, new);
        };
        let node = self.at_path[from].1;
        self.conn
            .prepare_cached("UPDATE file_path SET to_seq = ?2 WHERE file = ?1 AND to_seq IS NULL")?
            .execute(params![file, seq])?;
        let (via, sim) = if r.exact {
            ("rename-exact", None)
        } else {
            ("rename-similar", Some(r.similarity))
        };
        self.conn
            .prepare_cached(
                "INSERT INTO file_path (file, path, from_seq, via, similarity)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![file, to, seq, via, sim])?;
        let old = self.alive[&file].1;
        if old != new {
            self.version(file, seq, old.is_some(), new)?;
        }
        self.alive.insert(file, (to.to_owned(), new));
        self.at_path.insert(to.to_owned(), (file, node));
        self.moved.insert(file);
        Ok(())
    }

    fn version(
        &mut self,
        file: i64,
        seq: i64,
        close: bool,
        new: Option<ObjectId>,
    ) -> rusqlite::Result<()> {
        if close {
            self.conn
                .prepare_cached(
                    "UPDATE file_version SET to_seq = ?2 WHERE file = ?1 AND to_seq IS NULL",
                )?
                .execute(params![file, seq])?;
        }
        if let Some(n) = new {
            self.conn
                .prepare_cached(
                    "INSERT INTO file_version (file, blob, from_seq) VALUES (?1, ?2, ?3)",
                )?
                .execute(params![file, n.as_bytes(), seq])?;
        }
        Ok(())
    }

    /// An addition, modification or deletion at `path`. A path that comes back takes the lineage
    /// that last held it, unless that lineage lives on elsewhere (it moved): lineages never merge
    /// two files alive at once.
    pub(crate) fn file_change(
        &mut self,
        repo: i64,
        seq: i64,
        path: &str,
        new: Option<ObjectId>,
    ) -> rusqlite::Result<()> {
        if let Some(file) = self.file_at(path) {
            let old = self.alive[&file].1;
            self.version(file, seq, old.is_some(), new)?;
            match new {
                Some(_) => {
                    self.alive.get_mut(&file).expect("alive").1 = new;
                }
                None => {
                    self.conn
                        .prepare_cached(
                            "UPDATE file_path SET to_seq = ?2 WHERE file = ?1 AND to_seq IS NULL",
                        )?
                        .execute(params![file, seq])?;
                    self.alive.remove(&file);
                }
            }
            return Ok(());
        }
        let Some(n) = new else {
            return Ok(());
        };
        let (file, node) = match self.at_path.get(path) {
            Some(&(f, node)) if !self.alive.contains_key(&f) => (f, node),
            _ => {
                self.conn
                    .prepare_cached("INSERT INTO file (repo) VALUES (?1)")?
                    .execute([repo])?;
                let file = self.conn.last_insert_rowid();
                self.conn
                    .prepare_cached("INSERT INTO node (kind, file) VALUES ('file', ?1)")?
                    .execute([file])?;
                (file, self.conn.last_insert_rowid())
            }
        };
        self.conn
            .prepare_cached("INSERT INTO file_path (file, path, from_seq) VALUES (?1, ?2, ?3)")?
            .execute(params![file, path, seq])?;
        self.version(file, seq, false, Some(n))?;
        self.alive.insert(file, (path.to_owned(), Some(n)));
        self.at_path.insert(path.to_owned(), (file, node));
        self.moved.insert(file);
        Ok(())
    }

    fn link_id(&mut self, nl: NodeLink) -> rusqlite::Result<i64> {
        if let Some(id) = self.links.get(&nl) {
            return Ok(*id);
        }
        self.conn
            .prepare_cached(
                "INSERT INTO link (kind, src, dst) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            )?
            .execute(params![nl.0, nl.1, nl.2])?;
        let id = self
            .conn
            .prepare_cached("SELECT id FROM link WHERE kind = ?1 AND src = ?2 AND dst = ?3")?
            .query_row(params![nl.0, nl.1, nl.2], |r| r.get(0))?;
        self.links.insert(nl, id);
        Ok(id)
    }

    /// A link that closed at `seq`. One that opened at or before the last flush has its open row:
    /// it gets its end.
    fn close(&mut self, nl: NodeLink, seq: i64) -> rusqlite::Result<()> {
        let Some(from) = self.open.remove(&nl) else {
            return Ok(());
        };
        let id = self.link_id(nl)?;
        if from > self.written {
            self.conn
                .prepare_cached(
                    "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)",
                )?
                .execute(params![id, from, seq])?;
        } else {
            self.conn
                .prepare_cached(
                    "UPDATE interval SET to_seq = ?2 WHERE link = ?1 AND to_seq IS NULL",
                )?
                .execute(params![id, seq])?;
        }
        Ok(())
    }

    /// Before a batch commits: the open rows of links that opened since the last flush, each
    /// repository's indexed commit and the watermark, so the batch is a complete index.
    pub(crate) fn flush(&mut self, seq: i64) -> rusqlite::Result<()> {
        let mut open: Vec<(NodeLink, i64)> = self
            .open
            .iter()
            .filter(|(_, from)| **from > self.written)
            .map(|(l, f)| (*l, *f))
            .collect();
        open.sort_unstable();
        for (nl, from) in open {
            let id = self.link_id(nl)?;
            self.conn
                .prepare_cached("INSERT INTO interval (link, from_seq) VALUES (?1, ?2)")?
                .execute(params![id, from])?;
        }
        self.written = seq;
        for (repo, (at, sha)) in self.last.drain() {
            self.conn
                .prepare_cached(
                    "UPDATE repo SET indexed_commit = ?2, indexed_seq = ?3, head = ?2 WHERE id = ?1",
                )?
                .execute(params![repo, sha.as_bytes(), at])?;
        }
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('watermark_seq', ?1)",
            )?
            .execute([seq])?;
        Ok(())
    }

    /// Facts of each blob, once.
    pub(crate) fn facts<'f>(
        &mut self,
        facts: impl Iterator<Item = (&'f (ObjectId, &'static str), &'f Arc<BlobFacts>)>,
    ) -> rusqlite::Result<()> {
        let mut kinds: HashMap<&str, i64> = HashMap::new();
        let mut rows: Vec<_> = facts.collect();
        rows.sort_by_key(|((o, k), _)| (*o, *k));
        for ((oid, kind), f) in rows {
            let ex = match kinds.get(kind) {
                Some(id) => *id,
                None => {
                    let id = extractor_id(self.conn, kind)?;
                    kinds.insert(kind, id);
                    id
                }
            };
            self.conn
                .prepare_cached(
                    "INSERT INTO fact (blob, extractor, payload) VALUES (?1, ?2, ?3)
                     ON CONFLICT DO NOTHING",
                )?
                .execute(params![oid.as_bytes(), ex, f.encode()])?;
        }
        Ok(())
    }
}

/// The extractor row of a kind at this binary's version, made when missing.
pub(crate) fn extractor_id(conn: &Connection, kind: &str) -> rusqlite::Result<i64> {
    let version = extractor_of(kind).version();
    conn.execute(
        "INSERT INTO extractor (name, version) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![kind, version],
    )?;
    conn.query_row(
        "SELECT id FROM extractor WHERE name = ?1 AND version = ?2",
        params![kind, version],
        |r| r.get(0),
    )
}

/// `s` cut to `max` bytes on a character boundary.
fn cut_to(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A specifier as stored: anything that looks like a secret masked (XS-10), then cut to
/// [`SPEC_BYTES`]. Masking comes first so a secret can't be cut in half and slip past the rules;
/// only the first 4 KiB is read, far more than is kept.
pub(crate) fn cut(s: &str) -> String {
    let masked = cartograph_core::secrets::mask(cut_to(s, 4096));
    cut_to(&masked, SPEC_BYTES).to_owned()
}
