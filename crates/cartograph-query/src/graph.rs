//! `graph(GraphQuery) -> GraphSlice` (SRV-04), first cut: one `at=` slice at file level.
//!
//! ponytail: phase 1 indexes files and import links only, so the slice is the live files and the
//! live links at one step, scanned from SQLite (5 ms for 115k rows in phase 0). Roll-ups to
//! module and service, `scope`, `range=`, stored positions and the per-generation interval cache
//! are phase 3; until then x, y are 0 and the client lays the slice out itself. The slice is built
//! in memory, not streamed from the cursor: the response cap bounds it.

use std::collections::HashMap;

use cartograph_api::types::{
    ConfidenceLevel, GraphParams, GraphSlice, LinkColumns, LinkKind, Lod, NodeColumns, NodeKind,
};
use cartograph_core::{LinkId, NodeId};
use cartograph_store::rusqlite::{self, OptionalExtension};

use crate::resolve::{link_id, node_id};
use crate::{At, QueryError, Snapshot};

/// Response cap: above it the request fails with `too_large` (SRV-04).
pub const MAX_NODES: usize = 100_000;
pub const MAX_LINKS: usize = 400_000;

/// A checked graph request.
#[derive(Debug, Clone)]
pub struct GraphQuery {
    pub at: At,
    /// None keeps every kind.
    pub kinds: Option<Vec<LinkKind>>,
    pub min_confidence: ConfidenceLevel,
    pub repo: Option<String>,
}

fn later(what: &str, when: &str) -> QueryError {
    QueryError::bad(
        format!("{what} isn't available yet"),
        format!("this index has files and import links only; {what} arrives with {when}"),
        "ask for lod=file with at=",
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
        match p.lod {
            None | Some(Lod::File) => {}
            Some(_) => return Err(later("a rolled-up level of detail", "phase 3 (SRV-04)")),
        }
        if p.scope.is_some() {
            return Err(later("scope", "roll-ups in phase 3 (SRV-04)"));
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
            kinds,
            // H2: scores under 0.30 are hidden by default, and every level starts at 0.30.
            min_confidence: p.min_confidence.unwrap_or(ConfidenceLevel::Guessed),
            repo: p.repo.as_ref().map(|r| r.0.clone()),
        })
    }
}

impl Snapshot<'_> {
    pub fn graph(&self, q: &GraphQuery) -> Result<GraphSlice, QueryError> {
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
        let narrower = "ask for fewer link kinds (kinds=) or one repo (repo=)";

        let mut nodes = NodeColumns {
            id: Vec::new(),
            kind: Vec::new(),
            name: Vec::new(),
            parent: Vec::new(),
            x: Vec::new(),
            y: Vec::new(),
            size: Vec::new(),
            children: Vec::new(),
        };
        let mut index: HashMap<i64, u32> = HashMap::new();
        // repo= keeps that repo's files; links with an end outside them are dropped below.
        let mut st = self.conn().prepare_cached(
            "SELECT n.id, fp.path FROM file_path fp JOIN node n ON n.file = fp.file
             JOIN file f ON f.id = fp.file
             WHERE fp.from_seq <= ?1 AND (fp.to_seq IS NULL OR fp.to_seq > ?1)
               AND (?2 IS NULL OR f.repo IN (SELECT id FROM repo WHERE name = ?2))
             ORDER BY fp.path",
        )?;
        let mut rows = st.query(rusqlite::params![seq, q.repo])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let i = nodes.id.len();
            if i >= MAX_NODES {
                return Err(QueryError::TooLarge {
                    why: format!("more than {MAX_NODES} files are live at this commit"),
                    narrower: narrower.into(),
                });
            }
            self.check_rows(i + 1, narrower)?;
            index.insert(id, i as u32);
            nodes.id.push(NodeId(node_id(id)));
            nodes.kind.push(NodeKind::File);
            // The repo-relative path: unique, and what a file is known by.
            nodes.name.push(r.get(1)?);
            nodes.parent.push(-1);
            nodes.x.push(0.0);
            nodes.y.push(0.0);
            nodes.size.push(1.0);
            nodes.children.push(0);
        }

        let mut links = LinkColumns {
            id: Vec::new(),
            from: Vec::new(),
            to: Vec::new(),
            kind: Vec::new(),
            level: Vec::new(),
            score: Vec::new(),
            weight: Vec::new(),
            added_seq: Vec::new(),
            removed_seq: Vec::new(),
        };
        // Phase 1 links are parsed imports: both ends anchored in the tree, so exact at 1.0.
        let (level, score) = (ConfidenceLevel::Exact, 1.0);
        let keep = |k: LinkKind| q.kinds.as_ref().is_none_or(|ks| ks.contains(&k));
        if keep(LinkKind::Imports) && rank(level) >= rank(q.min_confidence) {
            let mut st = self.conn().prepare_cached(
                "SELECT l.id, l.src, l.dst, i.from_seq, i.to_seq
                 FROM interval i JOIN link l ON l.id = i.link
                 WHERE i.from_seq <= ?1 AND (i.to_seq IS NULL OR i.to_seq > ?1)
                 ORDER BY l.id",
            )?;
            let mut rows = st.query([seq])?;
            while let Some(r) = rows.next()? {
                let (src, dst): (i64, i64) = (r.get(1)?, r.get(2)?);
                let (Some(&from), Some(&to)) = (index.get(&src), index.get(&dst)) else {
                    // With repo=, a link into another repo has its other end outside the slice.
                    // Otherwise the indexer only links files it indexed; a dangling end is its
                    // bug, not ours to paper over silently.
                    if q.repo.is_none() {
                        tracing::warn!(
                            src,
                            dst,
                            seq,
                            "a live link has an end that isn't a live file"
                        );
                    }
                    continue;
                };
                if links.id.len() >= MAX_LINKS {
                    return Err(QueryError::TooLarge {
                        why: format!("more than {MAX_LINKS} links are live at this commit"),
                        narrower: narrower.into(),
                    });
                }
                self.check_rows(nodes.id.len() + links.id.len() + 1, narrower)?;
                links.id.push(LinkId(link_id(r.get(0)?)));
                links.from.push(from);
                links.to.push(to);
                links.kind.push(LinkKind::Imports);
                links.level.push(level);
                links.score.push(score);
                links.weight.push(1);
                links.added_seq.push(r.get::<_, i64>(3)? as u64);
                links
                    .removed_seq
                    .push(r.get::<_, Option<i64>>(4)?.unwrap_or(-1));
            }
        }

        Ok(GraphSlice {
            lod: Lod::File,
            scope: None,
            at: Some(at),
            range: None,
            nodes,
            links,
        })
    }
}
