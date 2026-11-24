//! The query-adapter runtime and the built-in adapters (XS-03). An adapter is a folder:
//! `adapter.toml` (`name`, `version`, `language`, `files` globs, `requires` hints) and
//! `queries/*.scm`. It turns one blob into interface facts ([`crate::schema::BlobFact`]), which are
//! cached by (blob, adapter, version) in the fact cache, so a blob is parsed once however many
//! times it is read.
//!
//! # Capture names are the contract
//!
//! In a query, `@<kind>` (the wire name of a fact kind, as in `@http.call`) marks the node a fact
//! is about: its span is the fact's span and its text the evidence. `@<kind>.<attr>` (as in
//! `@http.call.url`) adds an attribute to the fact of that kind in the same match. A capture that
//! starts with `_` is only for predicates and is dropped. `(#set! <attr> "<value>")` adds a constant
//! attribute, which a capture of the same attribute overrides. Matches of one node are merged, so a
//! query for `fetch(url)` and a second for `fetch(url, { method })` give one fact.
//!
//! Predicates: `#eq?`, `#not-eq?`, `#any-of?`, `#match?` (compiled with the `regex` crate under a
//! size limit) and `#set!`. Any other is refused when the adapter loads.
//!
//! # Limits
//!
//! A hostile query or input can explode, so every run has a match limit (64k), a time budget (250 ms,
//! checked in the cursor's progress callback) and an input cap (1 MB, the parse cap). Only grammars
//! compiled into the binary are targeted. A run that hit a limit keeps what it found and is not
//! cached.
//!
//! # Cost
//!
//! An adapter whose `requires` hints appear nowhere in the file is skipped, and a file no adapter
//! applies to isn't parsed. All of a grammar's adapters share one parse and one cursor pass: their
//! queries are compiled as one.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cartograph_parse::syntax::{parse, span, text};
use regex::Regex;
use serde::Deserialize;
use tree_sitter::{Language, Node, Query, QueryCursor, QueryCursorOptions, StreamingIterator};

use crate::cache::FactCache;
use crate::schema::{
    AttrValue, Attrs, BlobFact, Capped, Evidence, EvidenceType, FactKind, MAX_ATTRS,
};

pub mod query;
mod value;

#[cfg(test)]
mod tests;

/// Run limits (XS-03). The defaults are the ones every run uses; tests lower them.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub match_limit: u32,
    pub time_budget: Duration,
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            match_limit: 65_536,
            time_budget: Duration::from_millis(250),
            max_bytes: cartograph_parse::MAX_BLOB_BYTES as usize,
        }
    }
}

/// The grammars adapters can target. Third-party grammars are never loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grammar {
    Ts,
    Tsx,
    Js,
}

impl Grammar {
    const ALL: [Grammar; 3] = [Self::Ts, Self::Tsx, Self::Js];

    /// The parser's key, and the cache key's grammar part.
    fn key(self) -> &'static str {
        match self {
            Self::Ts => "ts",
            Self::Tsx => "tsx",
            Self::Js => "js",
        }
    }

    fn parser_key(self) -> &'static str {
        match self {
            Self::Ts => "adapters/ts",
            Self::Tsx => "adapters/tsx",
            Self::Js => "adapters/js",
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Js => tree_sitter_javascript::LANGUAGE.into(),
        }
    }

    fn for_path(path: &str) -> Option<Self> {
        match path.rsplit_once('.')?.1 {
            "ts" | "mts" | "cts" => Some(Self::Ts),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::Js),
            _ => None,
        }
    }

    /// The grammars a manifest's `language` names.
    fn named(language: &str) -> Option<Vec<Self>> {
        Some(match language {
            "typescript" => vec![Self::Ts],
            "tsx" => vec![Self::Tsx],
            "javascript" => vec![Self::Js],
            // One query set for the whole family; the node types they share are most of them.
            "ecmascript" => Self::ALL.to_vec(),
            _ => return None,
        })
    }
}

fn language_of(key: &str) -> Language {
    Grammar::ALL
        .into_iter()
        .find(|g| g.parser_key() == key)
        .expect("a parser key comes from a grammar")
        .language()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    version: String,
    language: String,
    files: Vec<String>,
    #[serde(default)]
    requires: Vec<String>,
}

/// One loaded adapter.
pub struct Adapter {
    pub name: String,
    pub version: String,
    grammars: Vec<Grammar>,
    files: Vec<Regex>,
    requires: Vec<String>,
    scm: String,
}

/// Why an adapter didn't load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<query::QueryError> for LoadError {
    fn from(e: query::QueryError) -> Self {
        Self(e.0)
    }
}

const MAX_GLOBS: usize = 16;
const MAX_GLOB_BYTES: usize = 128;
const MAX_QUERY_FILES: usize = 16;

/// A glob as a regex: `**/` any folders (or none), `*` within a folder name, `?` one character.
fn glob(pattern: &str) -> Result<Regex, LoadError> {
    if pattern.len() > MAX_GLOB_BYTES {
        return Err(LoadError("glob too long".into()));
    }
    let mut re = String::from("^");
    let mut rest = pattern;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**/") {
            re.push_str("(?:.*/)?");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("**") {
            re.push_str(".*");
            rest = r;
        } else {
            let c = rest.chars().next().expect("not empty");
            match c {
                '*' => re.push_str("[^/]*"),
                '?' => re.push_str("[^/]"),
                c => re.push_str(&regex::escape(&c.to_string())),
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    re.push('$');
    Regex::new(&re).map_err(|e| LoadError(e.to_string()))
}

fn valid_name(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Adapter {
    /// An adapter from its `adapter.toml` and its `queries/*.scm` files (name, text). Everything
    /// about it is checked here, so a bad adapter fails at load and not at the first blob.
    pub fn load(manifest: &str, queries: &[(&str, &str)]) -> Result<Self, LoadError> {
        let m: Manifest = toml::from_str(manifest).map_err(|e| LoadError(e.to_string()))?;
        if !valid_name(&m.name) || m.version.is_empty() || m.version.len() > 32 {
            return Err(LoadError(format!("bad name or version: {}", m.name)));
        }
        let grammars = Grammar::named(&m.language)
            .ok_or_else(|| LoadError(format!("unknown language {}", m.language)))?;
        if m.files.is_empty() || m.files.len() > MAX_GLOBS || m.requires.len() > MAX_GLOBS {
            return Err(LoadError("files and requires take 1 to 16 entries".into()));
        }
        if queries.is_empty() || queries.len() > MAX_QUERY_FILES {
            return Err(LoadError("an adapter has 1 to 16 query files".into()));
        }
        let files = m.files.iter().map(|g| glob(g)).collect::<Result<_, _>>()?;
        let mut scm = String::new();
        for (_, src) in queries {
            query::check(src)?;
            scm.push_str(src);
            scm.push('\n');
        }
        let a = Self {
            name: m.name,
            version: m.version,
            grammars,
            files,
            requires: m.requires,
            scm,
        };
        // Compiling is the rest of the check: syntax, node types of every grammar, capture names.
        for g in &a.grammars {
            Compiled::new(*g, &[(0, &a)])?;
        }
        Ok(a)
    }

    fn applies_to(&self, grammar: Grammar, path: &str) -> bool {
        self.grammars.contains(&grammar) && self.files.iter().any(|f| f.is_match(path))
    }

    /// False when none of the `requires` hints appears in the file (no hints: always true).
    fn wanted(&self, bytes: &[u8]) -> bool {
        self.requires.is_empty()
            || self
                .requires
                .iter()
                .any(|h| bytes.windows(h.len().max(1)).any(|w| w == h.as_bytes()))
    }

    fn cache_key(&self, grammar: Grammar) -> String {
        format!("adapter-{}/{}", self.name, grammar.key())
    }
}

/// What a capture name means in a query.
#[derive(Clone)]
enum Role {
    Ignore,
    Outer(FactKind),
    Attr(FactKind, String),
}

/// `http.call.url` is the `url` of an `http.call`; `http.call` is the fact itself.
fn role(name: &str) -> Result<Role, LoadError> {
    if name.starts_with('_') {
        return Ok(Role::Ignore);
    }
    // The longest kind name that the capture starts with.
    let kind = FactKind::ALL
        .into_iter()
        .filter(|k| {
            let k = k.as_str();
            name == k || name.strip_prefix(k).is_some_and(|r| r.starts_with('.'))
        })
        .max_by_key(|k| k.as_str().len())
        .ok_or_else(|| LoadError(format!("capture @{name} names no fact kind")))?;
    match name
        .strip_prefix(kind.as_str())
        .and_then(|r| r.strip_prefix('.'))
    {
        None => Ok(Role::Outer(kind)),
        Some(attr) if !attr.is_empty() => Ok(Role::Attr(kind, attr.replace('.', "_"))),
        Some(_) => Err(LoadError(format!("capture @{name} has an empty attribute"))),
    }
}

/// The queries of every adapter for one grammar, compiled as one query so a blob takes one pass.
struct Compiled {
    query: Query,
    roles: Vec<Role>,
    /// Per pattern: its adapter's index in the set, and its `#set!` constants.
    patterns: Vec<(usize, Vec<(String, String)>)>,
}

impl Compiled {
    /// `adapters` are (index in the set, adapter): a pattern reports the index.
    fn new(grammar: Grammar, adapters: &[(usize, &Adapter)]) -> Result<Self, LoadError> {
        let mut src = String::new();
        let mut starts = Vec::new();
        for (i, a) in adapters {
            starts.push((src.len(), *i));
            src.push_str(&a.scm);
        }
        let query = Query::new(&grammar.language(), &src).map_err(|e| LoadError(e.to_string()))?;
        let roles = query
            .capture_names()
            .iter()
            .map(|n| role(n))
            .collect::<Result<Vec<_>, _>>()?;
        let patterns = (0..query.pattern_count())
            .map(|p| {
                let at = query.start_byte_for_pattern(p);
                let adapter = starts
                    .iter()
                    .rev()
                    .find(|(s, _)| *s <= at)
                    .map_or(0, |(_, i)| *i);
                let consts = query
                    .property_settings(p)
                    .iter()
                    .filter_map(|s| Some((s.key.to_string(), s.value.as_ref()?.to_string())))
                    .collect();
                (adapter, consts)
            })
            .collect();
        Ok(Self {
            query,
            roles,
            patterns,
        })
    }
}

/// A fact being built from the matches of one node.
struct Draft {
    span: [u32; 4],
    evidence: String,
    /// Attribute -> (value, constant). A captured value replaces a constant.
    attrs: BTreeMap<String, (AttrValue, bool)>,
}

/// A node's source on one line, for evidence.
fn snippet(n: Node, src: &[u8]) -> String {
    text(n, src)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl Draft {
    fn finish(self, kind: FactKind) -> BlobFact {
        let literal = self.attrs.values().all(|(v, _)| value::is_literal(v));
        let attrs = self
            .attrs
            .into_iter()
            .take(MAX_ATTRS)
            .map(|(k, (v, _))| (Capped::truncated(&k), normalise(&k, v)))
            .collect();
        BlobFact {
            kind,
            span: crate::schema::FactSpan(self.span),
            attrs: Attrs::new(attrs).expect("taken to the cap"),
            evidence: Evidence {
                kind: if literal {
                    EvidenceType::Literal
                } else {
                    EvidenceType::Template
                },
                text: Capped::truncated(&self.evidence),
            },
        }
    }
}

/// HTTP methods are upper-case, and Express's `all` is every method.
fn normalise(key: &str, v: AttrValue) -> AttrValue {
    match (key, &v) {
        ("method", AttrValue::Str(s)) => {
            let up = s.as_str().to_ascii_uppercase();
            AttrValue::Str(Capped::truncated(if up == "ALL" { "ANY" } else { &up }))
        }
        _ => v,
    }
}

/// What one pass over a blob found, per adapter (by index in the set).
struct Pass {
    per_adapter: Vec<Vec<BlobFact>>,
    /// False when a limit stopped the pass: the facts are partial and are not cached.
    complete: bool,
}

/// The adapters, their queries compiled per grammar.
pub struct AdapterSet {
    adapters: Vec<Adapter>,
    compiled: Vec<(Grammar, Compiled)>,
}

/// The facts of one adapter for one blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterFacts {
    pub name: String,
    pub version: String,
    pub facts: Vec<BlobFact>,
}

/// What [`AdapterSet::extract`] returns for one blob.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// For every adapter that applies to the path, cached or new.
    pub facts: Vec<AdapterFacts>,
    /// True when the blob was parsed for this call; false when the cache answered (or nothing
    /// applied).
    pub parsed: bool,
}

impl AdapterSet {
    pub fn new(adapters: Vec<Adapter>) -> Result<Self, LoadError> {
        let mut compiled = Vec::new();
        for g in Grammar::ALL {
            let mine: Vec<(usize, &Adapter)> = adapters
                .iter()
                .enumerate()
                .filter(|(_, a)| a.grammars.contains(&g))
                .collect();
            if !mine.is_empty() {
                compiled.push((g, Compiled::new(g, &mine)?));
            }
        }
        Ok(Self { adapters, compiled })
    }

    /// The adapters embedded in the binary: express and fetch.
    pub fn builtin() -> &'static Self {
        static SET: OnceLock<AdapterSet> = OnceLock::new();
        SET.get_or_init(|| {
            let adapters = BUILTIN
                .iter()
                .map(|(manifest, queries)| Adapter::load(manifest, queries))
                .collect::<Result<_, _>>()
                .expect("built-in adapters load");
            Self::new(adapters).expect("built-in adapters compile")
        })
    }

    pub fn adapters(&self) -> &[Adapter] {
        &self.adapters
    }

    /// Facts for one blob, from the cache when every adapter that applies has an entry. `read`
    /// gives the blob's bytes and is called only on a miss. A pass that hit a limit isn't cached.
    pub fn extract(
        &self,
        cache: &mut FactCache,
        blob: &[u8],
        path: &str,
        read: impl FnOnce() -> Option<Vec<u8>>,
        limits: &Limits,
    ) -> Extracted {
        let Some(grammar) = Grammar::for_path(path) else {
            return Extracted::default();
        };
        let applies: Vec<usize> = (0..self.adapters.len())
            .filter(|&i| self.adapters[i].applies_to(grammar, path))
            .collect();
        if applies.is_empty() {
            return Extracted::default();
        }
        let mut have: Vec<Option<Vec<BlobFact>>> = applies
            .iter()
            .map(|&i| {
                let a = &self.adapters[i];
                cache.get_interface(blob, &a.cache_key(grammar), &a.version)
            })
            .collect();
        let mut parsed = false;
        if have.iter().any(Option::is_none) {
            let bytes = read().filter(|b| b.len() <= limits.max_bytes);
            let pass = bytes.as_deref().and_then(|b| self.pass(grammar, b, limits));
            parsed = pass.is_some();
            let (mut per_adapter, complete) = match pass {
                Some(p) => (p.per_adapter, p.complete),
                // Too big, unreadable or past the time limit: nothing to say about it, now or
                // later, unless the bytes couldn't be read.
                None => (Vec::new(), bytes.is_some()),
            };
            let active: Vec<bool> = self
                .adapters
                .iter()
                .map(|a| bytes.as_deref().is_some_and(|b| a.wanted(b)))
                .collect();
            let mut rows = Vec::new();
            for (slot, &i) in have.iter_mut().zip(&applies) {
                if slot.is_some() {
                    continue;
                }
                let mut facts = if active[i] && !per_adapter.is_empty() {
                    std::mem::take(&mut per_adapter[i])
                } else {
                    Vec::new()
                };
                facts.sort();
                facts.dedup();
                if complete {
                    rows.push((i, facts.clone()));
                }
                *slot = Some(facts);
            }
            // ponytail: a failed cache write only costs a re-parse next time.
            let keys: Vec<(String, &str, &[BlobFact])> = rows
                .iter()
                .map(|(i, f)| {
                    let a = &self.adapters[*i];
                    (a.cache_key(grammar), a.version.as_str(), f.as_slice())
                })
                .collect();
            let _ =
                cache.put_interface_many(keys.iter().map(|(k, v, f)| (blob, k.as_str(), *v, *f)));
        }
        Extracted {
            facts: applies
                .iter()
                .zip(have)
                .map(|(&i, f)| AdapterFacts {
                    name: self.adapters[i].name.clone(),
                    version: self.adapters[i].version.clone(),
                    facts: f.unwrap_or_default(),
                })
                .collect(),
            parsed,
        }
    }

    /// Parses `bytes` once and runs every adapter's queries in one cursor pass. `None` when the
    /// parse hit its time limit or no adapter's hints are in the file.
    fn pass(&self, grammar: Grammar, bytes: &[u8], limits: &Limits) -> Option<Pass> {
        let compiled = &self.compiled.iter().find(|(g, _)| *g == grammar)?.1;
        let active: Vec<bool> = self
            .adapters
            .iter()
            .map(|a| a.grammars.contains(&grammar) && a.wanted(bytes))
            .collect();
        if !active.iter().any(|&a| a) {
            return None;
        }
        let tree = parse(grammar.parser_key(), language_of, bytes)?;
        let start = Instant::now();
        let timed_out = Cell::new(false);
        let mut over = |_: &tree_sitter::QueryCursorState| {
            if start.elapsed() > limits.time_budget {
                timed_out.set(true);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let mut cursor = QueryCursor::new();
        cursor.set_match_limit(limits.match_limit);
        let mut drafts: BTreeMap<(usize, FactKind, usize, usize), Draft> = BTreeMap::new();
        {
            let mut matches = cursor.matches_with_options(
                &compiled.query,
                tree.root_node(),
                bytes,
                QueryCursorOptions::new().progress_callback(&mut over),
            );
            while let Some(m) = matches.next() {
                let (adapter, consts) = &compiled.patterns[m.pattern_index];
                if !active[*adapter] {
                    continue;
                }
                let mut outer = Vec::new();
                let mut attrs = Vec::new();
                for c in m.captures() {
                    match &compiled.roles[c.index as usize] {
                        Role::Ignore => {}
                        Role::Outer(k) => outer.push((*k, c.node)),
                        Role::Attr(k, name) => attrs.push((*k, name, c.node)),
                    }
                }
                for (kind, node) in outer {
                    let d = drafts
                        .entry((*adapter, kind, node.start_byte(), node.end_byte()))
                        .or_insert_with(|| Draft {
                            span: {
                                let s = span(node);
                                [s.start_line, s.start_col, s.end_line, s.end_col]
                            },
                            evidence: snippet(node, bytes),
                            attrs: BTreeMap::new(),
                        });
                    for (k, v) in consts {
                        d.attrs
                            .entry(k.clone())
                            .or_insert_with(|| (AttrValue::Str(Capped::truncated(v)), true));
                    }
                    for (_, name, n) in attrs.iter().filter(|(k, _, _)| *k == kind) {
                        d.attrs
                            .insert((*name).clone(), (value::value_of(*n, bytes), false));
                    }
                }
            }
        }
        let complete = !timed_out.get() && !cursor.did_exceed_match_limit();
        let mut per_adapter = vec![Vec::new(); self.adapters.len()];
        for ((adapter, kind, _, _), d) in drafts {
            per_adapter[adapter].push(d.finish(kind));
        }
        Some(Pass {
            per_adapter,
            complete,
        })
    }
}

/// (adapter.toml, [(query file name, text)]) for each built-in adapter.
type Embedded = (&'static str, &'static [(&'static str, &'static str)]);

static BUILTIN: &[Embedded] = &[
    (
        include_str!("../adapters/express/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/express/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/fetch/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/fetch/queries/calls.scm"),
        )],
    ),
];
