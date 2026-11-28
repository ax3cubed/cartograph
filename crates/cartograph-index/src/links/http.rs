//! HTTP links (XS-04): the path-template normaliser, a segment trie per service and method, and
//! the resolver that keeps call sites linked to routes as sites come and go.
//!
//! Everything here is pure: sites and host aliases go in as a [`Delta`], link events come out
//! through a [`LinkSink`]. The tries are rebuilt from facts on every run and never stored; the
//! index keeps only the intervals (see `crate::http`).
//!
//! Route patterns that embed a regex are never executed: a segment with regex syntax, or a whole
//! pattern anchored with `^`/`$`, is read as its literal leading segments followed by `{*}`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

/// Caps (XS-04): segments per template, bytes per template.
pub const MAX_SEGMENTS: usize = 64;
pub const MAX_TEMPLATE: usize = 2048;
const MAX_METHOD: usize = 16;
/// More candidates than this is `ambiguous`.
pub const MAX_CANDIDATES: usize = 3;
/// How an expression the code doesn't spell out is carried inside a raw string.
pub const EXPR: char = '\0';

/// One path segment of a template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Seg {
    Lit(String),
    /// `{}`: exactly one segment.
    Param,
    /// `{*}`: one or more segments, always last.
    Rest,
}

/// A method and a path template: `get /orders/{}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Template {
    pub method: String,
    pub segs: Vec<Seg>,
}

impl Template {
    /// `/orders/{}`.
    pub fn path(&self) -> String {
        path_string(&self.segs)
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path())
    }
}

fn path_string(segs: &[Seg]) -> String {
    if segs.is_empty() {
        return "/".into();
    }
    let mut s = String::new();
    for g in segs {
        s.push('/');
        match g {
            Seg::Lit(l) => s.push_str(l),
            Seg::Param => s.push_str("{}"),
            Seg::Rest => s.push_str("{*}"),
        }
    }
    s
}

fn cut_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut e = max;
    while !s.is_char_boundary(e) {
        e -= 1;
    }
    &s[..e]
}

/// Replaces each balanced `${...}` with an expression marker, so a `/` inside one doesn't split.
fn fold_exprs(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut skip_to = 0;
    for (i, c) in raw.char_indices() {
        if i < skip_to {
            continue;
        }
        if c == '$' && raw[i + 1..].starts_with('{') {
            let mut depth = 0usize;
            let mut end = None;
            for (j, d) in raw[i + 1..].char_indices() {
                match d {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1 + j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(e) = end {
                out.push(EXPR);
                skip_to = e + 1;
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn classify(s: &str) -> Seg {
    if s.contains(EXPR) {
        return Seg::Param;
    }
    if s.contains('*') || s.starts_with("[...") || s.starts_with("[[...") {
        return Seg::Rest;
    }
    if let Some(inner) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        let tail = inner.split_once(':').map(|(_, t)| t);
        let rest = match tail {
            Some(t) => matches!(t, ".*" | ".+" | "path" | "(.*)" | ".*?"),
            None => inner.starts_with('+') || inner.ends_with('+') || inner.ends_with("..."),
        };
        return if rest { Seg::Rest } else { Seg::Param };
    }
    if let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        return if inner.starts_with("path:") {
            Seg::Rest
        } else {
            Seg::Param
        };
    }
    if s.starts_with(':')
        || (s.starts_with('[') && s.ends_with(']'))
        || (s.contains('{') && s.contains('}'))
        || (s.contains('<') && s.contains('>'))
        || ["%s", "%d", "%v", "%q"].iter().any(|f| s.contains(f))
    {
        return Seg::Param;
    }
    // Regex syntax in a segment: never run it, read it as a catch-all.
    if s.chars().any(|c| "()|\\^$+?".contains(c)) {
        return Seg::Rest;
    }
    Seg::Lit(s.to_owned())
}

fn is_plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.~%-".contains(c))
}

/// The segments of a path pattern. `route`: the text is a route pattern, so `?` is not a query.
pub fn normalise_path(raw: &str, route: bool) -> Vec<Seg> {
    let raw = fold_exprs(cut_bytes(raw, 4 * MAX_TEMPLATE));
    let mut p = raw.split('#').next().unwrap_or("");
    if !route {
        p = p.split('?').next().unwrap_or("");
    }
    let anchored = route
        && (p.starts_with('^') || (p.ends_with('$') && !p.ends_with("{$}")) || p.contains("(?"));
    let mut segs = Vec::new();
    if anchored {
        let p = p.trim_start_matches('^').trim_end_matches('$');
        for s in p.split('/').filter(|s| !s.is_empty()) {
            if is_plain(s) {
                segs.push(Seg::Lit(s.to_owned()));
            } else {
                segs.push(Seg::Rest);
                break;
            }
        }
    } else {
        for s in p.split('/').filter(|s| !s.is_empty() && *s != "{$}") {
            let g = classify(s);
            let end = g == Seg::Rest;
            segs.push(g);
            if end {
                break;
            }
        }
    }
    if segs.len() > MAX_SEGMENTS {
        segs.truncate(MAX_SEGMENTS - 1);
        segs.push(Seg::Rest);
    }
    while path_string(&segs).len() > MAX_TEMPLATE {
        segs.pop();
        if segs.last() != Some(&Seg::Rest) {
            segs.push(Seg::Rest);
        }
        if path_string(&segs).len() > MAX_TEMPLATE {
            segs.pop();
        }
    }
    segs
}

fn method_of(m: &str) -> String {
    cut_bytes(&m.to_ascii_lowercase(), MAX_METHOD).to_owned()
}

/// A call: method, and a URL or path with scheme, host, query and fragment dropped.
pub fn normalise(method: &str, url: &str) -> Template {
    let (_, rest) = split_authority(&fold_exprs(cut_bytes(url, 4 * MAX_TEMPLATE)));
    Template {
        method: method_of(method),
        segs: normalise_path(&rest, false),
    }
}

/// A route pattern, as a framework writes it.
pub fn normalise_route(method: &str, pattern: &str) -> Template {
    Template {
        method: method_of(method),
        segs: normalise_path(pattern, true),
    }
}

/// A mount prefix joined onto a route's segments (`/api` + `/orders/{}`).
pub fn join_mount(prefix: &[Seg], segs: &[Seg]) -> Vec<Seg> {
    let mut v: Vec<Seg> = prefix.to_vec();
    if v.last() == Some(&Seg::Rest) {
        return v;
    }
    v.extend_from_slice(segs);
    v.truncate(MAX_SEGMENTS);
    v
}

/// `scheme://authority` split from the rest (which starts at `/`, `?` or `#`, or is empty).
fn split_authority(s: &str) -> (Option<String>, String) {
    if let Some(p) = s.find("://") {
        let scheme = &s[..p];
        if scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c) || c == EXPR)
        {
            let after = &s[p + 3..];
            let e = after.find(['/', '?', '#']).unwrap_or(after.len());
            return (Some(after[..e].to_owned()), after[e..].to_owned());
        }
    }
    (None, s.to_owned())
}

/// A host an alias can name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Host {
    pub name: String,
    pub port: Option<u16>,
}

/// The host and port of `user@Orders.:8080`: credentials dropped, lower-cased, no trailing dot.
/// `None` for an empty or bracketed host, or a port that isn't a number.
pub fn parse_host(a: &str) -> Option<Host> {
    let a = a.rsplit_once('@').map_or(a, |(_, h)| h);
    if a.starts_with('[') {
        return None;
    }
    let (h, port) = match a.rsplit_once(':') {
        Some((h, p)) => (h, Some(p.parse::<u16>().ok()?)),
        None => (a, None),
    };
    let name = h.trim_end_matches('.').to_ascii_lowercase();
    (!name.is_empty()).then_some(Host { name, port })
}

/// What a call site's URL says about where it goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub host: Option<Host>,
    pub segs: Vec<Seg>,
    /// The URL starts with something the code doesn't spell out (a base URL variable): the path
    /// is only a suffix of the real one.
    pub prefix_unknown: bool,
}

/// The target of a URL built from parts: `(is_expression, text)`.
pub fn call_target(parts: &[(bool, &str)]) -> Option<Target> {
    let mut s = String::new();
    for (expr, t) in parts {
        if *expr {
            s.push(EXPR);
        } else {
            s.push_str(t);
        }
    }
    let s = fold_exprs(cut_bytes(&s, 4 * MAX_TEMPLATE));
    if s.is_empty() {
        return None;
    }
    let (auth, rest) = split_authority(&s);
    let (host, unknown, rest) = match auth {
        Some(a) if a.contains(EXPR) => (None, true, rest),
        Some(a) => (Some(parse_host(&a)?), false, rest),
        None if s.starts_with(EXPR) => {
            let i = s.find('/')?;
            (None, true, s[i..].to_owned())
        }
        None => (None, false, rest),
    };
    let segs = normalise_path(&rest, false);
    if unknown && segs.is_empty() {
        return None;
    }
    Some(Target {
        host,
        segs,
        prefix_unknown: unknown,
    })
}

/// How a call's segments meet a route's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Match {
    /// Same segment count, every literal equal.
    Full,
    /// A literal against `{*}`.
    Partial,
    /// A full match of a call with no host, and the only candidate in the workspace.
    Unique,
}

/// Direct match of a call against a route, with the number of literal segments that agreed.
/// The trie must give the same answers.
pub fn match_segs(call: &[Seg], route: &[Seg]) -> Option<(Match, usize)> {
    let mut lits = 0;
    for (i, r) in route.iter().enumerate() {
        let c = call.get(i)?;
        if *r == Seg::Rest {
            let full = call.len() == i + 1 && *c == Seg::Rest;
            return Some((if full { Match::Full } else { Match::Partial }, lits));
        }
        if *c == Seg::Rest {
            return Some((Match::Partial, lits));
        }
        if let (Seg::Lit(a), Seg::Lit(b)) = (c, r) {
            if a != b {
                return None;
            }
            lits += 1;
        }
    }
    (call.len() == route.len()).then_some((Match::Full, lits))
}

/// A call whose prefix is unknown against a route: its segments must be the route's last ones.
pub fn match_suffix(call: &[Seg], route: &[Seg]) -> Option<usize> {
    if call.len() > route.len() || route.last() == Some(&Seg::Rest) {
        return None;
    }
    match match_segs(call, &route[route.len() - call.len()..]) {
        // At least one literal must agree, or any route of that length would do.
        Some((Match::Full, lits)) if lits > 0 => Some(lits),
        _ => None,
    }
}

/// Which kind of site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Call,
    Route,
}

/// A place in a file: its lineage, span and kind. Stable while the code stays put.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SiteKey {
    pub file: i64,
    pub span: [u32; 4],
    /// 0 for a site no environment decides; else the environment id plus one (XS-09).
    pub env: u8,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub key: SiteKey,
    pub service: i64,
    pub method: String,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSite {
    pub key: SiteKey,
    pub service: i64,
    pub method: String,
    pub segs: Vec<Seg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    Call(CallSite),
    Route(RouteSite),
}

/// A host name that reaches a service (XS-02).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Alias {
    pub host: String,
    pub port: Option<u16>,
    pub service: i64,
}

/// What one commit changed. Removes apply before adds.
#[derive(Debug, Default, Clone)]
pub struct Delta {
    pub add: Vec<Site>,
    pub remove: Vec<SiteKey>,
    pub alias_add: Vec<Alias>,
    pub alias_remove: Vec<Alias>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    Ambiguous,
    NotFound,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ambiguous => "ambiguous",
            Self::NotFound => "not-found",
        }
    }
}

/// A call linked to a route.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pair {
    pub call: SiteKey,
    pub route: SiteKey,
}

/// What a resolver reports as sites change.
pub trait LinkSink {
    fn open(&mut self, pair: &Pair, how: Match);
    fn close(&mut self, pair: &Pair);
    fn unresolved(&mut self, call: &SiteKey, reason: Reason);
    fn unresolved_closed(&mut self, call: &SiteKey);
}

/// A resolver the replay of a run feeds, one commit at a time.
pub trait LinkResolver {
    type Delta;
    fn apply(&mut self, seq: u64, delta: &Self::Delta, sink: &mut dyn LinkSink);
}

type Found = Vec<(SiteKey, Match, usize)>;

#[derive(Default)]
struct Node {
    lit: HashMap<String, Node>,
    param: Option<Box<Node>>,
    end: BTreeSet<SiteKey>,
    rest: BTreeSet<SiteKey>,
}

impl Node {
    fn insert(&mut self, segs: &[Seg], key: &SiteKey) {
        match segs.first() {
            None => {
                self.end.insert(key.clone());
            }
            Some(Seg::Rest) => {
                self.rest.insert(key.clone());
            }
            Some(Seg::Lit(l)) => self
                .lit
                .entry(l.clone())
                .or_default()
                .insert(&segs[1..], key),
            Some(Seg::Param) => self
                .param
                .get_or_insert_with(Default::default)
                .insert(&segs[1..], key),
        }
    }

    fn remove(&mut self, segs: &[Seg], key: &SiteKey) {
        match segs.first() {
            None => {
                self.end.remove(key);
            }
            Some(Seg::Rest) => {
                self.rest.remove(key);
            }
            Some(Seg::Lit(l)) => {
                if let Some(n) = self.lit.get_mut(l) {
                    n.remove(&segs[1..], key);
                    if n.is_empty() {
                        self.lit.remove(l);
                    }
                }
            }
            Some(Seg::Param) => {
                if let Some(n) = self.param.as_mut() {
                    n.remove(&segs[1..], key);
                    if n.is_empty() {
                        self.param = None;
                    }
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.lit.is_empty() && self.param.is_none() && self.end.is_empty() && self.rest.is_empty()
    }

    fn all(&self, lits: usize, out: &mut Found) {
        for k in self.end.iter().chain(&self.rest) {
            out.push((k.clone(), Match::Partial, lits));
        }
        for n in self.lit.values().chain(self.param.as_deref()) {
            n.all(lits, out);
        }
    }

    fn walk(&self, call: &[Seg], lits: usize, out: &mut Found) {
        let Some(c) = call.first() else {
            for k in &self.end {
                out.push((k.clone(), Match::Full, lits));
            }
            return;
        };
        let only = call.len() == 1 && *c == Seg::Rest;
        for k in &self.rest {
            let m = if only { Match::Full } else { Match::Partial };
            out.push((k.clone(), m, lits));
        }
        match c {
            Seg::Rest => {
                for n in self.lit.values().chain(self.param.as_deref()) {
                    n.all(lits, out);
                }
            }
            Seg::Lit(l) => {
                if let Some(n) = self.lit.get(l) {
                    n.walk(&call[1..], lits + 1, out);
                }
                if let Some(n) = &self.param {
                    n.walk(&call[1..], lits, out);
                }
            }
            Seg::Param => {
                for n in self.lit.values().chain(self.param.as_deref()) {
                    n.walk(&call[1..], lits, out);
                }
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct State {
    pairs: BTreeMap<SiteKey, Match>,
    unresolved: Option<Reason>,
}

type KeySet = HashMap<(String, String), HashSet<SiteKey>>;

fn toggle(set: &mut KeySet, k: (String, String), key: SiteKey, add: bool) {
    if add {
        set.entry(k).or_default().insert(key);
    } else if let Some(s) = set.get_mut(&k) {
        s.remove(&key);
        if s.is_empty() {
            set.remove(&k);
        }
    }
}

/// The HTTP resolver. Call sites and routes live in memory; a delta re-resolves only the calls
/// whose (method, first literal segment) key a changed route or call touches.
#[derive(Default)]
pub struct HttpResolver {
    calls: HashMap<SiteKey, CallSite>,
    routes: HashMap<SiteKey, RouteSite>,
    /// (service, method) -> trie of that service's routes.
    tries: HashMap<(i64, String), Node>,
    /// (method, literal segment anywhere in the route) -> routes, for calls with an unknown prefix.
    by_lit: KeySet,
    /// method -> services with a route of it.
    services: HashMap<String, BTreeSet<i64>>,
    /// The reverse index: (method, first literal segment, "" when not a literal) -> calls with a
    /// known prefix.
    full_calls: KeySet,
    by_method: HashMap<String, HashSet<SiteKey>>,
    /// (method, first literal segment) -> calls with an unknown prefix.
    suffix_calls: KeySet,
    host_calls: HashMap<String, HashSet<SiteKey>>,
    aliases: HashMap<String, BTreeSet<Alias>>,
    state: HashMap<SiteKey, State>,
}

fn first_lit(segs: &[Seg]) -> String {
    match segs.first() {
        Some(Seg::Lit(l)) => l.clone(),
        _ => String::new(),
    }
}

fn lits_of(segs: &[Seg]) -> impl Iterator<Item = &String> {
    segs.iter().filter_map(|s| match s {
        Seg::Lit(l) => Some(l),
        _ => None,
    })
}

impl HttpResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every live link.
    pub fn live(&self) -> BTreeSet<(Pair, Match)> {
        let mut v = BTreeSet::new();
        for (c, s) in &self.state {
            for (r, m) in &s.pairs {
                let pair = Pair {
                    call: c.clone(),
                    route: r.clone(),
                };
                v.insert((pair, *m));
            }
        }
        v
    }

    /// Every call that is unresolved, with why.
    pub fn unresolved(&self) -> BTreeSet<(SiteKey, Reason)> {
        self.state
            .iter()
            .filter_map(|(k, s)| Some((k.clone(), s.unresolved?)))
            .collect()
    }

    fn index_call(&mut self, c: &CallSite, add: bool) {
        let key = c.key.clone();
        let m = c.method.clone();
        if c.target.prefix_unknown {
            if let Some(l) = lits_of(&c.target.segs).next() {
                toggle(&mut self.suffix_calls, (m, l.clone()), key.clone(), add);
            }
        } else {
            let first = first_lit(&c.target.segs);
            toggle(&mut self.full_calls, (m.clone(), first), key.clone(), add);
            let set = self.by_method.entry(m).or_default();
            if add {
                set.insert(key.clone());
            } else {
                set.remove(&key);
            }
        }
        if let Some(h) = &c.target.host {
            let set = self.host_calls.entry(h.name.clone()).or_default();
            if add {
                set.insert(key);
            } else {
                set.remove(&key);
            }
        }
    }

    fn index_route(&mut self, r: &RouteSite, add: bool) {
        let tk = (r.service, r.method.clone());
        if add {
            self.tries.entry(tk).or_default().insert(&r.segs, &r.key);
            self.services
                .entry(r.method.clone())
                .or_default()
                .insert(r.service);
        } else if let Some(t) = self.tries.get_mut(&tk) {
            t.remove(&r.segs, &r.key);
            if t.is_empty() {
                self.tries.remove(&tk);
                if let Some(s) = self.services.get_mut(&r.method) {
                    s.remove(&r.service);
                }
            }
        }
        for l in lits_of(&r.segs) {
            toggle(
                &mut self.by_lit,
                (r.method.clone(), l.clone()),
                r.key.clone(),
                add,
            );
        }
    }

    /// The calls a route of this shape could change the answer for.
    fn affected(&self, r: &RouteSite, into: &mut BTreeSet<SiteKey>) {
        let methods: Vec<&String> = if r.method == "all" {
            self.by_method.keys().collect()
        } else {
            vec![&r.method]
        };
        let first = first_lit(&r.segs);
        for m in methods {
            if first.is_empty() {
                into.extend(self.by_method.get(m).into_iter().flatten().cloned());
            } else {
                for f in [first.as_str(), ""] {
                    let k = (m.clone(), f.to_owned());
                    into.extend(self.full_calls.get(&k).into_iter().flatten().cloned());
                }
            }
            for l in lits_of(&r.segs) {
                let k = (m.clone(), l.clone());
                into.extend(self.suffix_calls.get(&k).into_iter().flatten().cloned());
            }
        }
    }

    /// The services a host (and port) reaches: the best-ranked aliases, ones that declare the
    /// port first.
    fn host_services(&self, h: &Host) -> BTreeSet<i64> {
        let Some(set) = self.aliases.get(&h.name) else {
            return BTreeSet::new();
        };
        let hits: Vec<&Alias> = set
            .iter()
            .filter(|a| h.port.is_none() || a.port.is_none() || a.port == h.port)
            .collect();
        let pinned = hits.iter().any(|a| a.port.is_some());
        hits.iter()
            .filter(|a| !pinned || a.port.is_some())
            .map(|a| a.service)
            .collect()
    }

    fn lookup(&self, service: i64, c: &CallSite, out: &mut Found) {
        for m in [c.method.as_str(), "all"] {
            if let Some(t) = self.tries.get(&(service, m.to_owned())) {
                t.walk(&c.target.segs, 0, out);
            }
        }
    }

    fn resolve(&self, c: &CallSite) -> State {
        let mut st = State::default();
        if c.target.prefix_unknown {
            let Some(l) = lits_of(&c.target.segs).next() else {
                return st;
            };
            let mut found = Vec::new();
            let mut seen = HashSet::new();
            for m in [c.method.as_str(), "all"] {
                let k = (m.to_owned(), l.clone());
                for k in self.by_lit.get(&k).into_iter().flatten() {
                    if !seen.insert(k.clone()) {
                        continue;
                    }
                    let r = &self.routes[k];
                    if let Some(n) = match_suffix(&c.target.segs, &r.segs) {
                        found.push((k.clone(), Match::Partial, n));
                    }
                }
            }
            return settle(st, found, false);
        }
        if let Some(h) = &c.target.host {
            let services = self.host_services(h);
            return match services.len() {
                0 => st,
                1 => {
                    let s = *services.iter().next().expect("one");
                    let mut found = Vec::new();
                    self.lookup(s, c, &mut found);
                    settle(st, best_per_service(found), true)
                }
                _ => {
                    st.unresolved = Some(Reason::Ambiguous);
                    st
                }
            };
        }
        let mut all = Vec::new();
        let mut done = BTreeSet::new();
        for m in [c.method.as_str(), "all"] {
            for s in self.services.get(m).into_iter().flatten() {
                if !done.insert(*s) {
                    continue;
                }
                let mut v = Vec::new();
                self.lookup(*s, c, &mut v);
                all.extend(best_per_service(v));
            }
        }
        let mut st = settle(st, all, false);
        if st.pairs.len() == 1 {
            for m in st.pairs.values_mut().filter(|m| **m == Match::Full) {
                *m = Match::Unique;
            }
        }
        st
    }
}

/// Within one service, the most specific matches of the best class.
fn best_per_service(mut v: Found) -> Found {
    v.sort();
    v.dedup_by(|a, b| a.0 == b.0);
    let Some(best) = v.iter().map(|x| (x.1, std::cmp::Reverse(x.2))).min() else {
        return v;
    };
    v.retain(|x| (x.1, std::cmp::Reverse(x.2)) == best);
    v
}

/// Keeps the best class of candidates when there are 1 to 3 of them; more is `ambiguous`. With a
/// service chosen by host, no candidate at all is `not-found`.
fn settle(mut st: State, mut found: Found, by_host: bool) -> State {
    if let Some(best) = found.iter().map(|x| x.1).min() {
        found.retain(|x| x.1 == best);
    }
    if found.is_empty() {
        if by_host {
            st.unresolved = Some(Reason::NotFound);
        }
    } else if found.len() > MAX_CANDIDATES {
        st.unresolved = Some(Reason::Ambiguous);
    } else {
        for (k, m, _) in found {
            st.pairs.insert(k, m);
        }
    }
    st
}

impl LinkResolver for HttpResolver {
    type Delta = Delta;

    fn apply(&mut self, _seq: u64, d: &Delta, sink: &mut dyn LinkSink) {
        let mut touched: BTreeSet<SiteKey> = BTreeSet::new();
        for a in &d.alias_remove {
            if let Some(s) = self.aliases.get_mut(&a.host) {
                s.remove(a);
            }
            touched.extend(self.host_calls.get(&a.host).into_iter().flatten().cloned());
        }
        for a in &d.alias_add {
            self.aliases
                .entry(a.host.clone())
                .or_default()
                .insert(a.clone());
            touched.extend(self.host_calls.get(&a.host).into_iter().flatten().cloned());
        }
        let mut gone: Vec<SiteKey> = Vec::new();
        for k in &d.remove {
            if let Some(c) = self.calls.remove(k) {
                self.index_call(&c, false);
                touched.remove(k);
                gone.push(k.clone());
            } else if let Some(r) = self.routes.remove(k) {
                self.index_route(&r, false);
                self.affected(&r, &mut touched);
            }
        }
        for s in &d.add {
            match s {
                Site::Call(c) => {
                    if let Some(old) = self.calls.insert(c.key.clone(), c.clone()) {
                        self.index_call(&old, false);
                    }
                    self.index_call(c, true);
                    touched.insert(c.key.clone());
                }
                Site::Route(r) => {
                    if let Some(old) = self.routes.insert(r.key.clone(), r.clone()) {
                        self.index_route(&old, false);
                        self.affected(&old, &mut touched);
                    }
                    self.index_route(r, true);
                    self.affected(r, &mut touched);
                }
            }
        }
        for k in gone {
            if !self.calls.contains_key(&k)
                && let Some(old) = self.state.remove(&k)
            {
                emit(&k, &old, &State::default(), sink);
            }
        }
        for k in touched {
            let Some(c) = self.calls.get(&k) else {
                continue;
            };
            let new = self.resolve(c);
            let old = self.state.get(&k).cloned().unwrap_or_default();
            if old != new {
                emit(&k, &old, &new, sink);
            }
            if new == State::default() {
                self.state.remove(&k);
            } else {
                self.state.insert(k, new);
            }
        }
    }
}

fn emit(call: &SiteKey, old: &State, new: &State, sink: &mut dyn LinkSink) {
    let pair = |r: &SiteKey| Pair {
        call: call.clone(),
        route: r.clone(),
    };
    for (r, m) in &old.pairs {
        if new.pairs.get(r) != Some(m) {
            sink.close(&pair(r));
        }
    }
    if old.unresolved.is_some() && old.unresolved != new.unresolved {
        sink.unresolved_closed(call);
    }
    for (r, m) in &new.pairs {
        if old.pairs.get(r) != Some(m) {
            sink.open(&pair(r), *m);
        }
    }
    if let Some(u) = new.unresolved
        && old.unresolved != new.unresolved
    {
        sink.unresolved(call, u);
    }
}

/// A sink that drops everything.
pub struct NullSink;

impl LinkSink for NullSink {
    fn open(&mut self, _: &Pair, _: Match) {}
    fn close(&mut self, _: &Pair) {}
    fn unresolved(&mut self, _: &SiteKey, _: Reason) {}
    fn unresolved_closed(&mut self, _: &SiteKey) {}
}

/// Links and unresolved calls, as a resolver holds them.
pub type Resolved = (BTreeSet<(Pair, Match)>, BTreeSet<(SiteKey, Reason)>);

/// The links and unresolved calls of a set of sites resolved in one go: what an incremental run
/// must equal.
pub fn from_scratch(sites: &[Site], aliases: &[Alias]) -> Resolved {
    let mut r = HttpResolver::new();
    let d = Delta {
        add: sites.to_vec(),
        alias_add: aliases.to_vec(),
        ..Default::default()
    };
    r.apply(0, &d, &mut NullSink);
    (r.live(), r.unresolved())
}

#[cfg(test)]
mod tests;
