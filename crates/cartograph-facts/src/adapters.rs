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
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use tree_sitter::{Language, Node, Query, QueryCursor, QueryCursorOptions, StreamingIterator};

use crate::cache::FactCache;
use crate::schema::{
    AttrValue, Attrs, BlobFact, Capped, Evidence, EvidenceType, FactKind, MAX_ATTRS,
};

pub mod query;
pub mod subprocess;
mod value;

#[cfg(test)]
mod tests;

/// The most bytes read of a blob: generated clients (XS-11) are exempt from the 1 MB parse cap up to
/// this, and only an adapter that names a larger `max_bytes` runs on a blob over its own.
pub const MAX_GENERATED_BYTES: usize = 8 << 20;

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
            max_bytes: MAX_GENERATED_BYTES,
        }
    }
}

/// The grammars adapters can target. Third-party grammars are never loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grammar {
    Ts,
    Tsx,
    Js,
    Go,
    Py,
    Php,
    Yaml,
}

impl Grammar {
    const ALL: [Grammar; 7] = [
        Self::Ts,
        Self::Tsx,
        Self::Js,
        Self::Go,
        Self::Py,
        Self::Php,
        Self::Yaml,
    ];

    /// The parser's key, and the cache key's grammar part.
    fn key(self) -> &'static str {
        match self {
            Self::Ts => "ts",
            Self::Tsx => "tsx",
            Self::Js => "js",
            Self::Go => "go",
            Self::Py => "py",
            Self::Php => "php",
            Self::Yaml => "yaml",
        }
    }

    fn parser_key(self) -> &'static str {
        match self {
            Self::Ts => "adapters/ts",
            Self::Tsx => "adapters/tsx",
            Self::Js => "adapters/js",
            Self::Go => "adapters/go",
            Self::Py => "adapters/py",
            Self::Php => "adapters/php",
            Self::Yaml => "adapters/yaml",
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Js => tree_sitter_javascript::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Py => tree_sitter_python::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        }
    }

    fn for_path(path: &str) -> Option<Self> {
        match path.rsplit_once('.')?.1 {
            "ts" | "mts" | "cts" => Some(Self::Ts),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::Js),
            "go" => Some(Self::Go),
            "py" => Some(Self::Py),
            "php" => Some(Self::Php),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }

    /// The grammars a manifest's `language` names.
    fn named(language: &str) -> Option<Vec<Self>> {
        Some(match language {
            "typescript" => vec![Self::Ts],
            "tsx" => vec![Self::Tsx],
            "javascript" => vec![Self::Js],
            "go" => vec![Self::Go],
            "python" => vec![Self::Py],
            "php" => vec![Self::Php],
            "yaml" => vec![Self::Yaml],
            // One query set for the whole family; the node types they share are most of them.
            "ecmascript" => vec![Self::Ts, Self::Tsx, Self::Js],
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
    /// The largest blob this adapter reads; the parse cap when absent.
    max_bytes: Option<usize>,
    /// An attribute whose value a blob's facts hold once: the first place is evidence enough.
    unique: Option<String>,
    /// Where a fact's path comes from the file's own path (XS-15): the first rule of a kind
    /// whose `match` the path fits gives the path, `$1` as in a regex replacement. An empty
    /// `path` drops the fact. The `prefix` kind is put before every path of the adapter's routes.
    #[serde(default)]
    path_rule: Vec<PathRule>,
    /// A regex over the text of a file: one that matches is a router file, and is flagged when no
    /// adapter found a route in it.
    router_import: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathRule {
    kind: String,
    #[serde(rename = "match")]
    pattern: String,
    path: String,
}

/// One loaded adapter.
pub struct Adapter {
    pub name: String,
    pub version: String,
    grammars: Vec<Grammar>,
    files: Vec<Regex>,
    requires: Vec<String>,
    max_bytes: usize,
    unique: Option<String>,
    path_rules: Vec<(String, Regex, String)>,
    router_import: Option<Regex>,
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
        let a = Self::parse(manifest, queries)?;
        // Compiling is the rest of the check: syntax, node types of every grammar, capture names.
        for g in &a.grammars {
            Compiled::new(*g, &[(0, &a)])?;
        }
        Ok(a)
    }

    /// [`Adapter::load`] without compiling the queries: the built-in adapters, which a test
    /// loads in full, so a process that never parses doesn't pay for them.
    fn parse(manifest: &str, queries: &[(&str, &str)]) -> Result<Self, LoadError> {
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
        let re = |p: &str| {
            RegexBuilder::new(p)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| LoadError(e.to_string()))
        };
        if m.path_rule.len() > MAX_GLOBS {
            return Err(LoadError("path_rule takes up to 16 entries".into()));
        }
        let path_rules = m
            .path_rule
            .into_iter()
            .map(|r| Ok((r.kind, re(&r.pattern)?, r.path)))
            .collect::<Result<_, LoadError>>()?;
        let router_import = m.router_import.as_deref().map(re).transpose()?;
        let mut scm = String::new();
        for (_, src) in queries {
            query::check(src)?;
            scm.push_str(src);
            scm.push('\n');
        }
        Ok(Self {
            name: m.name,
            version: m.version,
            grammars,
            files,
            requires: m.requires,
            max_bytes: m
                .max_bytes
                .unwrap_or(cartograph_parse::MAX_BLOB_BYTES as usize),
            unique: m.unique,
            path_rules,
            router_import,
            scm,
        })
    }

    fn applies_to(&self, grammar: Grammar, path: &str) -> bool {
        self.grammars.contains(&grammar) && self.files.iter().any(|f| f.is_match(path))
    }

    /// False when none of the `requires` hints appears in the file (no hints: always true).
    fn wanted(&self, bytes: &[u8]) -> bool {
        bytes.len() <= self.max_bytes
            && (self.requires.is_empty()
                || self
                    .requires
                    .iter()
                    .any(|h| bytes.windows(h.len().max(1)).any(|w| w == h.as_bytes())))
    }

    /// The path a file's location gives its facts of `kind` (`route`, `entrypoint`, or `prefix`):
    /// Next.js's `app/orders/[id]/route.ts` is `/orders/:id`. `[x]` and `[...x]` are parameters and
    /// `(group)` folders are not part of the path. None when no rule fits or the rule drops it.
    pub fn path_for(&self, kind: &str, file: &str) -> Option<String> {
        let (_, re, tpl) = self
            .path_rules
            .iter()
            .find(|(k, re, _)| k == kind && re.is_match(file))?;
        if tpl.is_empty() {
            return None;
        }
        let mut raw = String::new();
        re.captures(file)?.expand(tpl, &mut raw);
        let segs: Vec<String> = raw
            .split('/')
            .filter(|s| !s.is_empty() && !(s.starts_with('(') && s.ends_with(')')))
            .map(|s| match s.trim_matches(['[', ']']) {
                p if p.len() < s.len() => format!(":{}", p.trim_start_matches("...")),
                _ => s.to_owned(),
            })
            .collect();
        Some(format!("/{}", segs.join("/")))
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

/// What is read of a node's source for evidence, before it is masked and cut to 200 characters:
/// more than the cap, so a secret just inside the cut is seen whole.
const SNIPPET_READ: usize = 512;

/// A node's source on one line, for evidence, with secrets masked (XS-10).
fn snippet(n: Node, src: &[u8]) -> String {
    let one_line = text(n, src)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let head: String = one_line.chars().take(SNIPPET_READ).collect();
    cartograph_core::secrets::mask(&head).into_owned()
}

/// `s` without the query string and fragment of any URL it spells out: a queue URL carries an
/// account id and often a signature (XS-16).
fn strip_url_queries(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || "\"'`)}],".contains(c))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        out.push_str(url.split(['?', '#']).next().unwrap_or(url));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

impl Draft {
    fn finish(self, kind: FactKind) -> BlobFact {
        let events = matches!(kind, FactKind::EventPublish | FactKind::EventSubscribe);
        let attrs: BTreeMap<_, _> = self
            .attrs
            .into_iter()
            .take(MAX_ATTRS)
            .map(|(k, (v, _))| {
                let v = match v {
                    AttrValue::Str(s) if events && s.as_str().contains("://") => {
                        AttrValue::Str(Capped::truncated(&strip_url_queries(s.as_str())))
                    }
                    v => v,
                };
                (k.clone(), normalise(&k, v))
            })
            .collect();
        // The names a fact is bound to say nothing about how well its value is known.
        let literal = attrs
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "router" | "parent"))
            .all(|(_, v)| value::is_literal(v));
        let attrs = attrs
            .into_iter()
            .map(|(k, v)| (Capped::truncated(&k), v))
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
                text: Capped::truncated(&if events {
                    strip_url_queries(&self.evidence)
                } else {
                    self.evidence
                }),
            },
        }
    }
}

/// HTTP methods are upper-case, and Express's `all` is every method.
fn normalise(key: &str, v: AttrValue) -> AttrValue {
    match (key, &v) {
        ("method", AttrValue::Str(_) | AttrValue::Parts { .. }) => {
            // A method named by an identifier (`@Get`, `r.GET`) is as good as a string.
            let up = value::plain(&v).to_ascii_uppercase();
            AttrValue::Str(Capped::truncated(if up == "ALL" { "ANY" } else { &up }))
        }
        _ => v,
    }
}

/// The flag on a file that looks like it declares routes and none was found.
fn unrecognised() -> BlobFact {
    let attr = (
        Capped::truncated("kind"),
        AttrValue::Str(Capped::truncated("unrecognised-route-file")),
    );
    BlobFact {
        kind: FactKind::Entrypoint,
        span: crate::schema::FactSpan([0; 4]),
        attrs: Attrs::new(BTreeMap::from([attr])).expect("one attribute"),
        evidence: Evidence {
            kind: EvidenceType::Inferred,
            text: Capped::truncated("a router file with no route found"),
        },
    }
}

/// What one pass over a blob found, per adapter (by index in the set).
struct Pass {
    per_adapter: Vec<Vec<BlobFact>>,
    /// False when a limit stopped the pass: the facts are partial and are not cached.
    complete: bool,
}

/// The adapters, their queries compiled per grammar on the first blob of that grammar.
pub struct AdapterSet {
    adapters: Vec<Adapter>,
    compiled: Vec<(Grammar, OnceLock<Compiled>)>,
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
    pub fn new(adapters: Vec<Adapter>) -> Self {
        let compiled = Grammar::ALL
            .into_iter()
            .filter(|g| adapters.iter().any(|a| a.grammars.contains(g)))
            .map(|g| (g, OnceLock::new()))
            .collect();
        Self { adapters, compiled }
    }

    /// The adapters embedded in the binary.
    pub fn builtin() -> &'static Self {
        static SET: OnceLock<AdapterSet> = OnceLock::new();
        SET.get_or_init(|| Self::embedded(BUILTIN))
    }

    /// The adapters that read the names a file calls.
    pub fn refs() -> &'static Self {
        static SET: OnceLock<AdapterSet> = OnceLock::new();
        SET.get_or_init(|| Self::embedded(REFS))
    }

    fn embedded(list: &[Embedded]) -> Self {
        {
            let adapters = list
                .iter()
                .map(|(manifest, queries)| Adapter::parse(manifest, queries))
                .collect::<Result<_, _>>()
                .expect("built-in adapters load");
            Self::new(adapters)
        }
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
        // One row per blob for all the adapters that apply, so a blob costs one read however many
        // adapters there are: keyed by the adapters and their versions, so a new one misses.
        let key = format!(
            "adapters/{}/{}",
            grammar.key(),
            applies
                .iter()
                .map(|&i| self.adapters[i].name.as_str())
                .collect::<Vec<_>>()
                .join("+")
        );
        let version = applies
            .iter()
            .map(|&i| self.adapters[i].version.as_str())
            .collect::<Vec<_>>()
            .join("+");
        let mut parsed = false;
        let have = cache
            .get_adapters(blob, &key, &version)
            .filter(|h| h.len() == applies.len());
        let have = match have {
            Some(h) => h,
            None => {
                let bytes = read().filter(|b| b.len() <= limits.max_bytes);
                let pass = bytes
                    .as_deref()
                    .and_then(|b| self.pass(grammar, path, b, limits));
                parsed = pass.is_some();
                let (mut per_adapter, complete) = match pass {
                    Some(p) => (p.per_adapter, p.complete),
                    // Too big, unreadable or past the time limit: nothing to say about it, now or
                    // later, unless the bytes couldn't be read.
                    None => (Vec::new(), bytes.is_some()),
                };
                // A router file nobody found a route in (XS-15): looked for only when no adapter
                // found one.
                let routed = per_adapter
                    .iter()
                    .any(|f| f.iter().any(|f| f.kind == FactKind::Route));
                let text = (!routed)
                    .then(|| bytes.as_deref().map(String::from_utf8_lossy))
                    .flatten();
                let in_routes = path.split('/').rev().skip(1).any(|d| d == "routes");
                let owner = applies
                    .iter()
                    .find(|&&i| self.adapters[i].router_import.is_some());
                let mut have = Vec::with_capacity(applies.len());
                for &i in &applies {
                    let mut facts = per_adapter
                        .get_mut(i)
                        .map(std::mem::take)
                        .unwrap_or_default();
                    let a = &self.adapters[i];
                    if let (Some(t), true) = (
                        &text,
                        complete && bytes.as_ref().is_some_and(|b| b.len() <= a.max_bytes),
                    ) && (a.router_import.as_ref().is_some_and(|r| r.is_match(t))
                        || (owner == Some(&i) && in_routes))
                    {
                        facts.push(unrecognised());
                    }
                    facts.sort();
                    facts.dedup();
                    have.push(facts);
                }
                // ponytail: a failed cache write only costs a re-parse next time.
                if complete {
                    let _ = cache.put_adapters(blob, &key, &version, &have);
                }
                have
            }
        };
        Extracted {
            facts: applies
                .iter()
                .zip(have)
                .map(|(&i, facts)| AdapterFacts {
                    name: self.adapters[i].name.clone(),
                    version: self.adapters[i].version.clone(),
                    facts,
                })
                .collect(),
            parsed,
        }
    }

    /// Facts for a blob that the caller keeps itself: no cache is read or written.
    pub fn run(&self, path: &str, bytes: &[u8], limits: &Limits) -> Vec<AdapterFacts> {
        let Some(grammar) = Grammar::for_path(path) else {
            return Vec::new();
        };
        let Some(pass) = self.pass(grammar, path, bytes, limits) else {
            return Vec::new();
        };
        pass.per_adapter
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.adapters[*i].applies_to(grammar, path))
            .map(|(i, mut facts)| {
                facts.sort();
                facts.dedup();
                AdapterFacts {
                    name: self.adapters[i].name.clone(),
                    version: self.adapters[i].version.clone(),
                    facts,
                }
            })
            .collect()
    }

    /// Parses `bytes` once and runs every adapter's queries in one cursor pass. `None` when the
    /// parse hit its time limit or no adapter's hints are in the file.
    fn pass(&self, grammar: Grammar, path: &str, bytes: &[u8], limits: &Limits) -> Option<Pass> {
        let active: Vec<bool> = self
            .adapters
            .iter()
            .map(|a| a.applies_to(grammar, path) && a.wanted(bytes))
            .collect();
        if !active.iter().any(|&a| a) {
            return None;
        }
        // Compiled after the hints check, so a blob no adapter wants never pays for it.
        let compiled = self
            .compiled
            .iter()
            .find(|(g, _)| *g == grammar)?
            .1
            .get_or_init(|| {
                let mine: Vec<(usize, &Adapter)> = self
                    .adapters
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.grammars.contains(&grammar))
                    .collect();
                Compiled::new(grammar, &mine).expect("adapters compile")
            });
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
        let mut seen = std::collections::HashSet::new();
        for ((adapter, kind, _, _), d) in drafts {
            if let Some(attr) = &self.adapters[adapter].unique {
                let v = d.attrs.get(attr).map(|(v, _)| value::plain(v));
                if v.is_none_or(|v| !seen.insert((adapter, v))) {
                    continue;
                }
            }
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
        &[
            (
                "calls.scm",
                include_str!("../adapters/fetch/queries/calls.scm"),
            ),
            ("env.scm", include_str!("../adapters/fetch/queries/env.scm")),
        ],
    ),
    (
        include_str!("../adapters/codegen/adapter.toml"),
        &[
            (
                "generated.scm",
                include_str!("../adapters/codegen/queries/generated.scm"),
            ),
            (
                "aliases.scm",
                include_str!("../adapters/codegen/queries/aliases.scm"),
            ),
            (
                "grpc.scm",
                include_str!("../adapters/codegen/queries/grpc.scm"),
            ),
        ],
    ),
    (
        include_str!("../adapters/codegen-go/adapter.toml"),
        &[
            (
                "generated.scm",
                include_str!("../adapters/codegen-go/queries/generated.scm"),
            ),
            (
                "aliases.scm",
                include_str!("../adapters/codegen-go/queries/aliases.scm"),
            ),
            (
                "grpc.scm",
                include_str!("../adapters/codegen-go/queries/grpc.scm"),
            ),
        ],
    ),
    (
        include_str!("../adapters/events/adapter.toml"),
        &[(
            "brokers.scm",
            include_str!("../adapters/events/queries/brokers.scm"),
        )],
    ),
    (
        include_str!("../adapters/events-go/adapter.toml"),
        &[(
            "kafka.scm",
            include_str!("../adapters/events-go/queries/kafka.scm"),
        )],
    ),
    (
        include_str!("../adapters/nestjs/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/nestjs/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/nextjs/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/nextjs/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/nextjs-pages/adapter.toml"),
        &[(
            "pages.scm",
            include_str!("../adapters/nextjs-pages/queries/pages.scm"),
        )],
    ),
    (
        include_str!("../adapters/fastapi/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/fastapi/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/gin/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/gin/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/grpc/adapter.toml"),
        &[(
            "register.scm",
            include_str!("../adapters/grpc/queries/register.scm"),
        )],
    ),
    (
        include_str!("../adapters/k8s-cronjob/adapter.toml"),
        &[(
            "cronjob.scm",
            include_str!("../adapters/k8s-cronjob/queries/cronjob.scm"),
        )],
    ),
    (
        include_str!("../adapters/laravel/adapter.toml"),
        &[(
            "routes.scm",
            include_str!("../adapters/laravel/queries/routes.scm"),
        )],
    ),
    (
        include_str!("../adapters/go-http/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/go-http/queries/calls.scm"),
        )],
    ),
    (
        include_str!("../adapters/python-http/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/python-http/queries/calls.scm"),
        )],
    ),
    (
        include_str!("../adapters/guzzle/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/guzzle/queries/calls.scm"),
        )],
    ),
];

/// The adapters that say which names a file calls. They are not part of [`AdapterSet::builtin`]: the
/// index runs them only on the files that import a generated client (XS-11).
static REFS: &[Embedded] = &[
    (
        include_str!("../adapters/refs/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/refs/queries/calls.scm"),
        )],
    ),
    (
        include_str!("../adapters/refs-go/adapter.toml"),
        &[(
            "calls.scm",
            include_str!("../adapters/refs-go/queries/calls.scm"),
        )],
    ),
];
