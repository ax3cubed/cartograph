//! Search (SRV-09): `search(q, kinds, at, scope, limit)` over the index's FTS5 tables, shared by the
//! palette route, the Ask @ picker and MCP `find`. Ranking and quoting live in
//! `cartograph_store::search` (one implementation, AI-15): each word is quoted, so FTS syntax in
//! `q` is text; bm25 with name over split name over path, then fan-in.
//!
//! Word-prefix hits come first; when they don't fill the page, substring hits of three or more
//! characters follow (the palette's `includes`). Nothing matched: the nearest name, a prefix match
//! on the first five letters of the first word (board G05).

use cartograph_api::types::{SearchKind, SearchResult, SearchResults};
use cartograph_store::search::{Filter, search_in, substring};

use crate::{At, QueryError, Snapshot};

/// `q` is cut to this many characters (and 16 words by the store's quoting).
pub const MAX_QUERY: usize = 256;

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub q: String,
    /// None keeps every kind.
    pub kinds: Option<Vec<SearchKind>>,
    pub at: At,
    /// A node reference: only it and what lies below it.
    pub scope: Option<String>,
    pub limit: usize,
}

impl SearchQuery {
    /// Checks the HTTP parameters.
    pub fn from_params(
        q: Option<&str>,
        kinds: Option<&str>,
        at: Option<&str>,
        scope: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Self, QueryError> {
        let limit = crate::Page::parse(limit, None)?.limit;
        let kinds = kinds
            .map(|s| {
                s.split(',')
                    .filter(|k| !k.trim().is_empty())
                    .map(|k| {
                        serde_json::from_value(serde_json::Value::String(k.trim().into())).map_err(
                            |_| {
                                QueryError::bad(
                                    "kinds names an unknown kind",
                                    "kinds are service, module, file, symbol, concept and operation",
                                    "for example kinds=file,module",
                                )
                            },
                        )
                    })
                    .collect::<Result<Vec<SearchKind>, _>>()
            })
            .transpose()?;
        Ok(Self {
            q: q.unwrap_or_default().chars().take(MAX_QUERY).collect(),
            kinds,
            at: At::parse(at.unwrap_or("now"))?,
            scope: scope.filter(|s| !s.is_empty()).map(str::to_owned),
            limit,
        })
    }
}

/// The `node.kind` values a search kind covers. Symbols, concepts and operations aren't indexed
/// yet (stage 5 and the XS items), so they match nothing.
fn db_kinds(kinds: &[SearchKind]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for k in kinds {
        match k {
            SearchKind::Service => out.extend(["service", "library"]),
            SearchKind::Module => out.push("module"),
            SearchKind::File => out.push("file"),
            SearchKind::Symbol | SearchKind::Concept | SearchKind::Operation => {}
        }
    }
    out
}

impl Snapshot<'_> {
    pub fn search(&self, q: &SearchQuery) -> Result<SearchResults, QueryError> {
        let (c, _) = self.step(&q.at)?;
        let kinds = q.kinds.as_deref().map(db_kinds);
        if kinds.as_ref().is_some_and(Vec::is_empty) || q.q.trim().is_empty() {
            return Ok(SearchResults {
                results: Vec::new(),
                near: None,
            });
        }
        let scope = q
            .scope
            .as_deref()
            .filter(|s| *s != crate::WORKSPACE)
            .map(|s| self.resolve_row(s, c.seq))
            .transpose()?;
        let f = Filter {
            at: Some(c.seq as i64),
            kinds: kinds.as_deref().unwrap_or(&[]),
            scope,
        };
        let mut hits: Vec<i64> = search_in(self.conn(), &q.q, &f, q.limit)?
            .into_iter()
            .map(|h| h.node)
            .collect();
        if hits.len() < q.limit {
            for h in substring(self.conn(), &q.q, &f, q.limit)? {
                if hits.len() == q.limit {
                    break;
                }
                if !hits.contains(&h.node) {
                    hits.push(h.node);
                }
            }
        }
        let near = if hits.is_empty() {
            let first: String =
                q.q.split(|c: char| !c.is_alphanumeric())
                    .find(|w| !w.is_empty())
                    .unwrap_or_default()
                    .chars()
                    .take(5)
                    .collect();
            search_in(self.conn(), &first, &f, 1)?
                .first()
                .map(|h| self.search_result(h.node, c.seq))
                .transpose()?
        } else {
            None
        };
        Ok(SearchResults {
            results: hits
                .into_iter()
                .map(|n| self.search_result(n, c.seq))
                .collect::<Result<_, _>>()?,
            near,
        })
    }

    fn search_result(&self, row: i64, seq: u64) -> Result<SearchResult, QueryError> {
        let n = self.wire_node(row, seq)?;
        Ok(SearchResult {
            id: n.id,
            kind: match n.kind {
                cartograph_api::types::NodeKind::Module => SearchKind::Module,
                cartograph_api::types::NodeKind::File => SearchKind::File,
                _ => SearchKind::Service,
            },
            name: n.name,
            path: n.path,
            service: n.service,
        })
    }
}
