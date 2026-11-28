//! HTTP links in the index (XS-04): call sites linked to routes across services, with intervals.
//!
//! Runs last in a run's derived rows, because it needs what those build: each file's service
//! (`node.service`) and the host aliases (XS-02). It reads every stored interface fact (see
//! `crate::ifaces`) with the time each version of its file was at its path, feeds them commit by
//! commit to an [`HttpResolver`] and writes what comes out: a `link` of kind `http` from the
//! calling file's node to the route's file node, its intervals, the `evidence` rows (the call
//! site and the route site) and the `unresolved` rows (`ambiguous`, `not-found`).
//!
//! The resolver's tries live in memory for the length of the pass and are never stored. The pass
//! is rebuilt whole when its inputs key changes (any site's file version or path, any alias), so a
//! sync equals a rebuild by construction; the incremental resolver is what makes the whole pass a
//! single sweep over the timeline.
//!
//! Two files can be joined by several call and route pairs: the link counts them, opens when the
//! first appears and closes when the last goes, in the commit where that happens.
//!
//! Contracts (XS-08): the operations of the specs (`crate::contracts`) are routes of a second
//! resolver that holds the same calls, so a call is bound to the operations it consumes. An
//! operation and a parsed route with the same method and template are bound (`handled_by`); an
//! operation with no route is spec-only, and a route of a service that has a spec and is in none is
//! undocumented. A link whose call consumes an operation that its route handles has the contract
//! matched on both ends: that is the evidence that makes it exact, and the operation is a third
//! evidence row. Bindings are `binding` rows with intervals.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cartograph_facts::schema::{AttrValue, Attrs, EvidenceType, FactKind, Part};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, params};

use crate::config::{self, Env};
use crate::contracts::{self, codegen};
use crate::derive::key;
use crate::ifaces;
use crate::links::confidence::{self, EvidenceType as Ev};
use crate::links::http::{
    Alias, CallSite, Delta, HttpResolver, Kind, LinkResolver, LinkSink, Match, Pair, Reason,
    RouteSite, Site, SiteKey, Target, Template, call_target, normalise, normalise_route,
};
use crate::rows::cut;

/// Bump when what the rows hold changes, so every index rebuilds them.
const HTTP_VERSION: &str = "http 5";
/// The `meta` key holding the env keys the call sites read, so the pass key hashes only the values
/// those keys have (XS-09: a values-file change that touches no read key costs nothing).
const READ_KEYS: &str = "http_env_keys";

/// The config chain an environment resolved a call through.
#[derive(Clone)]
struct Cfg {
    env: i64,
    key: String,
    value: String,
    source: String,
}

/// What a site says about itself, for evidence rows.
#[derive(Clone)]
struct Info {
    file: i64,
    kind: &'static str,
    text: String,
    span: [u32; 4],
    extractor: String,
    /// The URL as the source spells it (`${expr}` for what it doesn't), for unresolved rows.
    raw: String,
    /// `get /orders/{}`.
    template: String,
    /// The call spells a host out.
    host: bool,
    /// The environment value the call's base came from.
    cfg: Option<Cfg>,
}

/// One version of a file at a path: its sites, over the overlap of the two intervals.
pub(crate) struct Row {
    pub(crate) file: i64,
    pub(crate) node: i64,
    pub(crate) service: i64,
    pub(crate) blob: Vec<u8>,
    pub(crate) from: i64,
    pub(crate) to: Option<i64>,
    pub(crate) payload: Vec<u8>,
}

/// The rows of the interface facts, or (`refs`) of the names the importers of generated clients call.
pub(crate) fn load(conn: &Connection, refs: bool) -> rusqlite::Result<Vec<Row>> {
    load_from(conn, ifaces::EXTRACTOR, refs)
}

/// [`load`] for the facts of another extractor (the table facts, XS-17).
pub(crate) fn load_from(
    conn: &Connection,
    extractor: &str,
    refs: bool,
) -> rusqlite::Result<Vec<Row>> {
    let mut st = conn.prepare_cached(
        "SELECT fp.file, n.id, coalesce(n.service, 0), fv.blob, fp.from_seq, fp.to_seq,
                fv.from_seq, fv.to_seq, fa.payload
         FROM (SELECT fa.blob, fa.payload FROM fact fa JOIN extractor e ON e.id = fa.extractor
                 WHERE e.name = ?1 AND NOT ?2
               UNION ALL SELECT blob, payload FROM blob_ref WHERE ?2) fa
           JOIN file_version fv ON fv.blob = fa.blob
           JOIN file_path fp ON fp.file = fv.file
           JOIN node n ON n.file = fv.file AND n.kind = 'file'
         ORDER BY fp.file, fv.blob, fp.from_seq, fv.from_seq",
    )?;
    let mut rows = st.query(params![extractor, refs])?;
    let mut out: Vec<Row> = Vec::new();
    while let Some(r) = rows.next()? {
        let from = r.get::<_, i64>(4)?.max(r.get(6)?);
        let to = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<i64>>(7)?) {
            (None, t) | (t, None) => t,
            (Some(a), Some(b)) => Some(a.min(b)),
        };
        if to.is_some_and(|t| t <= from) {
            continue;
        }
        let (file, blob): (i64, Vec<u8>) = (r.get(0)?, r.get(3)?);
        // A rename keeps the blob: one stretch, not two.
        if let Some(last) = out.last_mut()
            && last.file == file
            && last.blob == blob
            && last.to == Some(from)
        {
            last.to = to;
            continue;
        }
        out.push(Row {
            file,
            node: r.get(1)?,
            service: r.get(2)?,
            blob,
            from,
            to,
            payload: r.get(8)?,
        });
    }
    Ok(out)
}

/// One operation of a spec, over the time its version of the spec had it.
struct OpRow {
    id: i64,
    file: i64,
    service: i64,
    method: String,
    path: String,
    line: u32,
    from: i64,
    to: Option<i64>,
}

fn load_ops(conn: &Connection) -> rusqlite::Result<Vec<OpRow>> {
    let mut st = conn.prepare_cached(
        "SELECT o.id, c.file, coalesce(c.service, 0), o.method, o.path, o.line, o.from_seq, o.to_seq
         FROM operation o JOIN contract c ON c.id = o.contract WHERE c.kind = 'openapi'
         ORDER BY o.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(OpRow {
            id: r.get(0)?,
            file: r.get(1)?,
            service: r.get(2)?,
            method: r.get(3)?,
            path: r.get(4)?,
            line: r.get(5)?,
            from: r.get(6)?,
            to: r.get(7)?,
        })
    })?;
    rows.collect()
}

/// What an operation's site is: unique by its row.
fn op_key(o: &OpRow) -> SiteKey {
    SiteKey {
        file: o.file,
        span: [o.id as u32, 0, 0, 0],
        env: 0,
        kind: Kind::Route,
    }
}

struct Aliases {
    alias: Alias,
    from: i64,
    to: Option<i64>,
}

fn load_aliases(conn: &Connection) -> rusqlite::Result<Vec<Aliases>> {
    let mut st = conn.prepare_cached(
        "SELECT host, port, service_id, valid_from, valid_to FROM host_alias
         ORDER BY host, port, service_id, valid_from",
    )?;
    let rows = st.query_map([], |r| {
        Ok(Aliases {
            alias: Alias {
                host: r.get(0)?,
                port: r
                    .get::<_, Option<i64>>(1)?
                    .and_then(|p| u16::try_from(p).ok()),
                service: r.get(2)?,
            },
            from: r.get(3)?,
            to: r.get(4)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn text_of(v: &AttrValue) -> Option<String> {
    match v {
        AttrValue::Str(s) => Some(s.as_str().to_owned()),
        AttrValue::Parts { .. } => None,
    }
}

/// `(is_expression, text)` pieces of a value, and the same as source-like text.
pub(crate) fn pieces(v: &AttrValue) -> Vec<(bool, String)> {
    match v {
        AttrValue::Str(s) => vec![(false, s.as_str().to_owned())],
        AttrValue::Parts { parts } => parts
            .as_slice()
            .iter()
            .map(|p| match p {
                Part::Lit(s) => (false, s.as_str().to_owned()),
                Part::Expr(s) => (true, s.as_str().to_owned()),
            })
            .collect(),
    }
}

pub(crate) fn attr<'a>(a: &'a Attrs, k: &str) -> Option<&'a AttrValue> {
    a.get(k)
}

/// A call whose base is an environment variable: the sites it has depend on the environment.
struct EnvCall {
    key: SiteKey,
    /// The URL's pieces; the first is the expression that reads the variable.
    pieces: Vec<(bool, String)>,
    /// The variable.
    var: String,
    service: i64,
    method: String,
    /// What the call is when no environment decides, if it is anything.
    generic: Option<CallSite>,
    info: Info,
}

enum Prep {
    Site(Site, Box<Info>),
    Env(Box<EnvCall>),
}

/// The variable an expression reads: `process.env.X`, `process.env["X"]`, `import.meta.env.X`, or
/// a name the file bound to one.
pub(crate) fn env_var(expr: &str, bound: &HashMap<String, String>) -> Option<String> {
    let e = expr.trim().trim_end_matches('!');
    if let Some(n) = bound.get(e) {
        return Some(n.clone());
    }
    let rest = e
        .strip_prefix("process.env")
        .or_else(|| e.strip_prefix("import.meta.env"))?;
    let name = match rest.strip_prefix('.') {
        Some(n) => n,
        None => rest
            .strip_prefix('[')?
            .strip_suffix(']')?
            .trim_matches(['"', '\'', '`']),
    };
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then(|| name.to_owned())
}

/// The text of a one-expression or one-string value (an identifier the adapter captured).
pub(crate) fn name_of(v: &AttrValue) -> Option<String> {
    match pieces(v).as_slice() {
        [(_, t)] => Some(t.clone()),
        _ => None,
    }
}

/// The sites of one version of a file.
fn sites(row: &Row, out: &mut Vec<Prep>) {
    let Some(per) = ifaces::decode(&row.payload) else {
        return;
    };
    // The names the file binds to an env var, and its named clients' bases.
    let mut bound: HashMap<String, String> = HashMap::new();
    let mut clients: HashMap<String, Vec<(bool, String)>> = HashMap::new();
    for (_, _, facts) in &per {
        for f in facts {
            match f.kind {
                FactKind::EnvRead => {
                    if let (Some(k), Some(n)) = (
                        attr(&f.attrs, "key").and_then(name_of),
                        attr(&f.attrs, "name").and_then(name_of),
                    ) {
                        bound.insert(n, k);
                    }
                }
                FactKind::ClientBase => {
                    if let (Some(n), Some(u)) = (
                        attr(&f.attrs, "name").and_then(name_of),
                        attr(&f.attrs, "url"),
                    ) {
                        clients.insert(n, pieces(u));
                    }
                }
                _ => {}
            }
        }
    }
    for (name, version, facts) in per {
        for f in facts {
            let key = |kind| SiteKey {
                file: row.file,
                span: f.span.0,
                env: 0,
                kind,
            };
            let kind = match f.evidence.kind {
                EvidenceType::Literal => "literal",
                EvidenceType::Template => "template",
                EvidenceType::Inferred => "inferred",
            };
            let mk_info = |template: String, raw: String| Info {
                file: row.file,
                kind,
                // Masked again: the adapter masked what it found, this is the stored copy.
                text: cut(f.evidence.text.as_str()),
                span: f.span.0,
                extractor: format!("{name}@{version}"),
                raw,
                template,
                host: false,
                cfg: None,
            };
            match f.kind {
                FactKind::Route => {
                    let Some(method) = attr(&f.attrs, "method").and_then(text_of) else {
                        continue;
                    };
                    let method = method.to_ascii_lowercase();
                    // A mount (`app.use("/api", router)`) names no route of its own.
                    if method == "use" {
                        continue;
                    }
                    let Some(path) = attr(&f.attrs, "path") else {
                        continue;
                    };
                    let raw: String = pieces(path)
                        .iter()
                        .map(|(e, t)| if *e { "${}".to_owned() } else { t.clone() })
                        .collect();
                    let t = normalise_route(&method, &raw);
                    let info = mk_info(t.to_string(), raw);
                    out.push(Prep::Site(
                        Site::Route(RouteSite {
                            key: key(Kind::Route),
                            service: row.service,
                            method: t.method,
                            segs: t.segs,
                        }),
                        Box::new(info),
                    ));
                }
                FactKind::HttpCall => {
                    let Some(method) = attr(&f.attrs, "method").and_then(text_of) else {
                        continue;
                    };
                    let Some(url) = attr(&f.attrs, "url") else {
                        continue;
                    };
                    let mut ps = pieces(url);
                    // A call through a named client starts with the client's base; one through a
                    // client the file doesn't define isn't a call we know.
                    if let Some(c) = attr(&f.attrs, "client").and_then(name_of) {
                        match clients.get(&c) {
                            Some(base) => {
                                ps.splice(0..0, base.iter().cloned());
                            }
                            None if matches!(c.as_str(), "axios" | "got" | "ky") => {}
                            None => continue,
                        }
                    }
                    let var = ps
                        .first()
                        .filter(|(e, _)| *e)
                        .and_then(|(_, t)| env_var(t, &bound));
                    let borrowed: Vec<(bool, &str)> =
                        ps.iter().map(|(e, t)| (*e, t.as_str())).collect();
                    let target = call_target(&borrowed);
                    if target.is_none() && var.is_none() {
                        continue;
                    }
                    let raw: String = ps
                        .iter()
                        .map(|(e, t)| if *e { format!("${{{t}}}") } else { t.clone() })
                        .collect();
                    let method = method.to_ascii_lowercase();
                    let template = format!(
                        "{method} {}",
                        crate::links::http::normalise(&method, &raw).path()
                    );
                    let mut info = mk_info(template, raw);
                    info.host = target.as_ref().is_some_and(|t| t.host.is_some());
                    let generic = target.map(|target| CallSite {
                        key: key(Kind::Call),
                        service: row.service,
                        method: method.clone(),
                        target,
                    });
                    match var {
                        Some(var) => out.push(Prep::Env(Box::new(EnvCall {
                            key: key(Kind::Call),
                            pieces: ps,
                            var,
                            service: row.service,
                            method,
                            generic,
                            info,
                        }))),
                        None => {
                            if let Some(g) = generic {
                                out.push(Prep::Site(Site::Call(g), Box::new(info)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

type Link = (i64, i64);
type EvidenceRow = (i64, &'static str, String, [u32; 4], String, String);
type UnresolvedRow = (i64, String, u32, String, String, i64, Option<i64>);
/// (link, environment, key, value, source): what a link was resolved through.
type CfgKey = (Link, i64, String, String, String);

/// One commit's events, netted before they touch a link.
#[derive(Default)]
struct Step {
    open: Vec<(Pair, Match)>,
    close: Vec<Pair>,
    unres: Vec<(SiteKey, Reason)>,
    unres_closed: Vec<SiteKey>,
}

impl LinkSink for Step {
    fn open(&mut self, p: &Pair, how: Match) {
        self.open.push((p.clone(), how));
    }
    fn close(&mut self, p: &Pair) {
        self.close.push(p.clone());
    }
    fn unresolved(&mut self, c: &SiteKey, r: Reason) {
        self.unres.push((c.clone(), r));
    }
    fn unresolved_closed(&mut self, c: &SiteKey) {
        self.unres_closed.push(c.clone());
    }
}

/// Everything a pass produced, ready to write.
#[derive(Default)]
struct Out {
    /// Link -> its intervals, the last open when `to` is `None`.
    intervals: BTreeMap<Link, Vec<(i64, Option<i64>)>>,
    /// Link -> (file, kind, text, span, extractor, operation), deduplicated.
    evidence: BTreeMap<Link, BTreeSet<EvidenceRow>>,
    /// Link -> what the confidence engine scores it from.
    types: BTreeMap<Link, BTreeSet<Ev>>,
    /// (file, template, line, reason, raw, from, to)
    unresolved: Vec<UnresolvedRow>,
    /// Link -> the environments it holds in, bit i for environment id i.
    envs: BTreeMap<Link, u32>,
    /// What each link was resolved through, with the stretch it held: (what, from, to).
    cfg: Vec<(CfgKey, i64, Option<i64>)>,
}

/// A binding of the contract layer: what joins an operation (none for an undocumented route) to
/// the route or call at `site`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Bind {
    kind: &'static str,
    op: Option<SiteKey>,
    site: SiteKey,
}

/// The contract layer of a sweep; present only when some spec has operations.
#[derive(Default)]
struct Layer {
    /// The calls and the operations.
    resolver: HttpResolver,
    /// Live routes and operations: (service, `get /orders/{}`).
    routes: HashMap<SiteKey, (i64, String)>,
    ops: HashMap<SiteKey, (i64, String)>,
    /// Pairs the main resolver holds, and the operations each call consumes.
    pairs: BTreeSet<Pair>,
    consumes: HashMap<SiteKey, BTreeSet<SiteKey>>,
    open: HashMap<Bind, usize>,
    rows: Vec<(Bind, i64, Option<i64>)>,
}

struct Sweep {
    resolver: HttpResolver,
    layer: Option<Layer>,
    info: HashMap<SiteKey, Info>,
    /// What the sites that changed this commit said before it.
    prev: HashMap<SiteKey, Info>,
    node_of: HashMap<i64, i64>,
    counts: HashMap<Link, i64>,
    open_at: HashMap<Link, usize>,
    unres_open: HashMap<SiteKey, (Reason, i64, usize)>,
    cfg_counts: HashMap<CfgKey, i64>,
    cfg_open: HashMap<CfgKey, usize>,
    out: Out,
}

impl Sweep {
    fn step(&mut self, seq: i64, d: &Delta, ops: &Delta) {
        let mut s = Step::default();
        self.resolver.apply(seq as u64, d, &mut s);
        let mut dirty = false;
        if let Some(l) = &mut self.layer {
            let is_call = |k: &SiteKey| k.kind == Kind::Call;
            let sd = Delta {
                add: d
                    .add
                    .iter()
                    .filter(|x| matches!(x, Site::Call(_)))
                    .chain(&ops.add)
                    .cloned()
                    .collect(),
                remove: d
                    .remove
                    .iter()
                    .filter(|k| is_call(k))
                    .chain(&ops.remove)
                    .cloned()
                    .collect(),
                alias_add: d.alias_add.clone(),
                alias_remove: d.alias_remove.clone(),
            };
            let mut t = Step::default();
            l.resolver.apply(seq as u64, &sd, &mut t);
            for p in &t.close {
                if let Some(v) = l.consumes.get_mut(&p.call) {
                    v.remove(&p.route);
                }
            }
            for (p, _) in &t.open {
                l.consumes
                    .entry(p.call.clone())
                    .or_default()
                    .insert(p.route.clone());
            }
            for k in &d.remove {
                dirty |= l.routes.remove(k).is_some();
            }
            for k in &ops.remove {
                dirty |= l.ops.remove(k).is_some();
            }
            for x in &d.add {
                if let Site::Route(r) = x {
                    let t = Template {
                        method: r.method.clone(),
                        segs: r.segs.clone(),
                    };
                    l.routes.insert(r.key.clone(), (r.service, t.to_string()));
                    dirty = true;
                }
            }
            for x in &ops.add {
                if let Site::Route(r) = x {
                    let t = Template {
                        method: r.method.clone(),
                        segs: r.segs.clone(),
                    };
                    l.ops.insert(r.key.clone(), (r.service, t.to_string()));
                    dirty = true;
                }
            }
            for p in &s.close {
                l.pairs.remove(p);
            }
            for (p, _) in &s.open {
                l.pairs.insert(p.clone());
            }
            dirty |= !(t.open.is_empty()
                && t.close.is_empty()
                && s.open.is_empty()
                && s.close.is_empty());
        }
        let link =
            |p: &Pair, n: &HashMap<i64, i64>| Some((*n.get(&p.call.file)?, *n.get(&p.route.file)?));
        let mut before: HashMap<Link, i64> = HashMap::new();
        for p in &s.close {
            if let Some(l) = link(p, &self.node_of).filter(|l| l.0 != l.1) {
                let was = self.prev.get(&p.call).or_else(|| self.info.get(&p.call));
                if let Some(c) = was.and_then(|i| i.cfg.as_ref()) {
                    let k: CfgKey = (l, c.env, c.key.clone(), c.value.clone(), c.source.clone());
                    let n = self.cfg_counts.entry(k.clone()).or_default();
                    *n -= 1;
                    if *n == 0
                        && let Some(i) = self.cfg_open.remove(&k)
                    {
                        self.out.cfg[i].2 = Some(seq);
                    }
                }
                before
                    .entry(l)
                    .or_insert_with(|| self.counts.get(&l).copied().unwrap_or(0));
                *self.counts.entry(l).or_default() -= 1;
            }
        }
        for (p, how) in &s.open {
            let Some(l) = link(p, &self.node_of).filter(|l| l.0 != l.1) else {
                continue;
            };
            before
                .entry(l)
                .or_insert_with(|| self.counts.get(&l).copied().unwrap_or(0));
            *self.counts.entry(l).or_default() += 1;
            let types = self.out.types.entry(l).or_default();
            if let Some(c) = self.info.get(&p.call).and_then(|i| i.cfg.as_ref()) {
                types.insert(Ev::EnvResolvedBase);
                *self.out.envs.entry(l).or_default() |= 1 << c.env;
                let k: CfgKey = (l, c.env, c.key.clone(), c.value.clone(), c.source.clone());
                let n = self.cfg_counts.entry(k.clone()).or_default();
                *n += 1;
                if *n == 1 {
                    self.out.cfg.push((k.clone(), seq, None));
                    self.cfg_open.insert(k, self.out.cfg.len() - 1);
                }
            }
            if let Some(i) = self.info.get(&p.call) {
                match i.kind {
                    "literal" => types.insert(Ev::LiteralCallSiteUrl),
                    "template" => types.insert(Ev::TemplateCallSiteUrl),
                    "generated" => types.insert(Ev::GeneratedClientSpec),
                    _ => false,
                };
                if i.host {
                    types.insert(Ev::HostAliasResolved);
                }
            }
            match how {
                Match::Partial => types.insert(Ev::PartialPathMatch),
                Match::Unique => {
                    types.insert(Ev::UniqueAcrossWorkspace);
                    types.insert(Ev::FullPathMatch);
                    types.insert(Ev::HandlerAnchored)
                }
                Match::Full => {
                    types.insert(Ev::FullPathMatch);
                    types.insert(Ev::HandlerAnchored)
                }
            };
            let ev = self.out.evidence.entry(l).or_default();
            for (k, partial) in [(&p.call, *how == Match::Partial), (&p.route, false)] {
                if let Some(i) = self.info.get(k) {
                    // A partial match is inferred from more than the call site's own text.
                    let kind = if partial { "inferred" } else { i.kind };
                    ev.insert((
                        i.file,
                        kind,
                        i.text.clone(),
                        i.span,
                        i.extractor.clone(),
                        i.template.clone(),
                    ));
                }
            }
        }
        let mut ls: Vec<_> = before.into_iter().collect();
        ls.sort_unstable();
        for (l, was) in ls {
            let now = self.counts.get(&l).copied().unwrap_or(0);
            if was == 0 && now > 0 {
                let v = self.out.intervals.entry(l).or_default();
                v.push((seq, None));
                self.open_at.insert(l, v.len() - 1);
            } else if was > 0
                && now == 0
                && let Some(i) = self.open_at.remove(&l)
            {
                self.out.intervals.get_mut(&l).expect("opened")[i].1 = Some(seq);
            }
        }
        // Unresolved calls: closed then reopened with the same reason is no change.
        let reopened: BTreeMap<&SiteKey, Reason> = s.unres.iter().map(|(k, r)| (k, *r)).collect();
        for k in &s.unres_closed {
            let Some((reason, _, _)) = self.unres_open.get(k) else {
                continue;
            };
            if reopened.get(k) == Some(reason) {
                continue;
            }
            let (_, _, i) = self.unres_open.remove(k).expect("open");
            self.out.unresolved[i].6 = Some(seq);
        }
        for (k, r) in &s.unres {
            if self.unres_open.get(k).is_some_and(|(x, _, _)| x == r) {
                continue;
            }
            let Some(i) = self.info.get(k) else { continue };
            self.out.unresolved.push((
                i.file,
                i.template.clone(),
                i.span[0],
                r.as_str().to_owned(),
                i.raw.clone(),
                seq,
                None,
            ));
            self.unres_open
                .insert(k.clone(), (*r, seq, self.out.unresolved.len() - 1));
        }
        if dirty {
            self.contracts(seq);
        }
    }

    /// Recomputes the bindings from what is live, and the links whose contract is matched on both
    /// ends.
    fn contracts(&mut self, seq: i64) {
        let Some(l) = &mut self.layer else { return };
        let mut routes_by: HashMap<&str, Vec<(&SiteKey, i64)>> = HashMap::new();
        for (k, (svc, tpl)) in &l.routes {
            routes_by.entry(tpl).or_default().push((k, *svc));
        }
        let mut handled: BTreeSet<(&SiteKey, &SiteKey)> = BTreeSet::new();
        for (ok, (osvc, tpl)) in &l.ops {
            for (rk, rsvc) in routes_by.get(tpl.as_str()).into_iter().flatten() {
                if *osvc == 0 || osvc == rsvc {
                    handled.insert((ok, rk));
                }
            }
        }
        let documented: HashSet<&SiteKey> = handled.iter().map(|(_, r)| *r).collect();
        let owners: HashSet<i64> = l
            .ops
            .values()
            .map(|(s, _)| *s)
            .filter(|s| *s != 0)
            .collect();
        let mut now: BTreeSet<Bind> = BTreeSet::new();
        for (o, r) in &handled {
            now.insert(Bind {
                kind: "handled_by",
                op: Some((*o).clone()),
                site: (*r).clone(),
            });
        }
        for (rk, (svc, _)) in &l.routes {
            if owners.contains(svc) && !documented.contains(rk) {
                now.insert(Bind {
                    kind: "undocumented",
                    op: None,
                    site: rk.clone(),
                });
            }
        }
        for (ck, ops) in &l.consumes {
            for ok in ops {
                now.insert(Bind {
                    kind: "consumes",
                    op: Some(ok.clone()),
                    site: ck.clone(),
                });
            }
        }
        // The pairs whose call consumes an operation that their route handles.
        let mut matched: Vec<(&Pair, &SiteKey)> = Vec::new();
        for p in &l.pairs {
            for ok in l.consumes.get(&p.call).into_iter().flatten() {
                if handled.contains(&(ok, &p.route)) {
                    matched.push((p, ok));
                }
            }
        }
        for (p, ok) in matched {
            let (Some(a), Some(b)) = (
                self.node_of.get(&p.call.file),
                self.node_of.get(&p.route.file),
            ) else {
                continue;
            };
            if a == b {
                continue;
            }
            let link = (*a, *b);
            self.out
                .types
                .entry(link)
                .or_default()
                .insert(Ev::ContractMatched);
            if let Some(i) = self.info.get(ok) {
                self.out.evidence.entry(link).or_default().insert((
                    i.file,
                    i.kind,
                    i.text.clone(),
                    i.span,
                    i.extractor.clone(),
                    i.template.clone(),
                ));
            }
        }
        let gone: Vec<Bind> = l
            .open
            .keys()
            .filter(|b| !now.contains(*b))
            .cloned()
            .collect();
        for b in gone {
            if let Some(i) = l.open.remove(&b) {
                l.rows[i].2 = Some(seq);
            }
        }
        for b in now {
            if !l.open.contains_key(&b) {
                l.rows.push((b.clone(), seq, None));
                l.open.insert(b, l.rows.len() - 1);
            }
        }
    }
}

/// The delta of each commit that changes a site or an alias.
type Timeline = (
    BTreeMap<i64, Delta>,
    HashMap<(SiteKey, i64), Info>,
    HashMap<i64, i64>,
    BTreeMap<i64, Delta>,
    // The variables the call sites read.
    BTreeSet<String>,
    // The calls no value reaches, as unresolved rows.
    Vec<UnresolvedRow>,
    Vec<codegen::Alias>,
    BTreeMap<i64, String>,
);

/// A config row as the resolver reads it: (source, value, from, to).
pub(crate) type Cv = (String, Option<String>, i64, Option<i64>);

/// Config rows by key, split into those of each environment and those of every one.
#[derive(Default)]
pub(crate) struct Values {
    pub(crate) own: HashMap<(String, i64), Vec<Cv>>,
    pub(crate) base: HashMap<String, Vec<Cv>>,
}

pub(crate) fn load_values(conn: &Connection) -> rusqlite::Result<Values> {
    let mut v = Values::default();
    let mut st = conn.prepare_cached(
        "SELECT key, env_id, value, source, valid_from, valid_to FROM config_value
         ORDER BY key, env_id, source, valid_from",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let key: String = r.get(0)?;
        let cv: Cv = (r.get(3)?, r.get(2)?, r.get(4)?, r.get(5)?);
        match r.get::<_, Option<i64>>(1)? {
            Some(e) => v.own.entry((key, e)).or_default().push(cv),
            None => v.base.entry(key).or_default().push(cv),
        }
    }
    Ok(v)
}

fn timeline(
    rows: &[Row],
    refs: &[Row],
    aliases: &[Aliases],
    ops: &[OpRow],
    envs: &[Env],
    values: &Values,
) -> Timeline {
    let mut deltas: BTreeMap<i64, Delta> = BTreeMap::new();
    // What each add says about itself, by (site, commit): a later version of the same place
    // replaces it in order.
    let mut infos = HashMap::new();
    let mut node_of = HashMap::new();
    let mut read = BTreeSet::new();
    let mut unresolved: Vec<UnresolvedRow> = Vec::new();
    let mut add = |deltas: &mut BTreeMap<i64, Delta>, site: Site, info: Info, from, to| {
        let k = match &site {
            Site::Call(c) => c.key.clone(),
            Site::Route(r) => r.key.clone(),
        };
        if let Some(to) = to {
            deltas.entry(to).or_default().remove.push(k.clone());
        }
        deltas.entry(from).or_default().add.push(site);
        infos.insert((k, from), info);
    };
    // Generated clients (XS-11): the files that say they are, and the functions that are operations.
    let mut generated = BTreeMap::new();
    let mut defs = Vec::new();
    for row in rows {
        node_of.insert(row.file, row.node);
        let mut found = codegen::Found::default();
        codegen::read(&row.payload, &mut found);
        if let Some(g) = found.generator
            && row.to.is_none()
        {
            generated.insert(row.node, g);
        }
        for (names, method, path, line) in found.defs {
            let (method, segs) = codegen::segs(&method, &path);
            defs.push(codegen::Def {
                file: row.file,
                service: row.service,
                names,
                method,
                segs,
                line,
                span: (row.from, row.to),
            });
        }
        let mut v = Vec::new();
        sites(row, &mut v);
        for prep in v {
            let e = match prep {
                Prep::Site(site, info) => {
                    add(&mut deltas, site, *info, row.from, row.to);
                    continue;
                }
                Prep::Env(e) => e,
            };
            read.insert(e.var.clone());
            // With no environment known, the call is what its own text says.
            if envs.is_empty() {
                if let Some(g) = e.generic {
                    add(&mut deltas, Site::Call(g), e.info, row.from, row.to);
                }
                continue;
            }
            let none = Vec::new();
            let base = values.base.get(&e.var).unwrap_or(&none);
            for env in envs {
                let own = values.own.get(&(e.var.clone(), env.id)).unwrap_or(&none);
                for (from, to, found) in config::effective(own, base, (row.from, row.to)) {
                    let Some((value, source)) = found else {
                        unresolved.push((
                            e.info.file,
                            e.info.template.clone(),
                            e.info.span[0],
                            format!("no value for {} in {}", e.var, env.name),
                            e.info.raw.clone(),
                            from,
                            to,
                        ));
                        continue;
                    };
                    let mut ps: Vec<(bool, &str)> =
                        e.pieces.iter().map(|(x, t)| (*x, t.as_str())).collect();
                    ps[0] = (false, value.as_str());
                    let Some(target) = call_target(&ps) else {
                        continue;
                    };
                    let mut info = e.info.clone();
                    info.host = target.host.is_some();
                    info.template = Template {
                        method: e.method.clone(),
                        segs: target.segs.clone(),
                    }
                    .to_string();
                    info.cfg = Some(Cfg {
                        env: env.id,
                        key: e.var.clone(),
                        value,
                        source,
                    });
                    let site = CallSite {
                        key: SiteKey {
                            env: env.id as u8 + 1,
                            ..e.key.clone()
                        },
                        service: e.service,
                        method: e.method.clone(),
                        target,
                    };
                    add(&mut deltas, Site::Call(site), info, from, to);
                }
            }
        }
    }
    for a in aliases {
        deltas
            .entry(a.from)
            .or_default()
            .alias_add
            .push(a.alias.clone());
        if let Some(to) = a.to {
            deltas
                .entry(to)
                .or_default()
                .alias_remove
                .push(a.alias.clone());
        }
    }
    let mut called = Vec::new();
    for row in refs {
        node_of.insert(row.file, row.node);
        let mut found = codegen::Found::default();
        codegen::read(&row.payload, &mut found);
        for (name, at, extractor) in found.refs {
            called.push(codegen::Ref {
                file: row.file,
                service: row.service,
                name,
                at,
                extractor,
                span: (row.from, row.to),
            });
        }
    }
    let views: Vec<codegen::Op> = ops
        .iter()
        .map(|o| {
            let (method, segs) = codegen::segs(&o.method, &o.path);
            codegen::Op {
                id: o.id,
                method,
                path: o.path.clone(),
                segs,
                span: (o.from, o.to),
            }
        })
        .collect();
    let (client_aliases, through) = codegen::link(&defs, &called, &views);
    for c in through {
        let t = normalise(&c.method, &c.path);
        let key = SiteKey {
            file: c.file,
            span: c.at,
            env: 0,
            kind: Kind::Call,
        };
        if infos.contains_key(&(key.clone(), c.span.0)) {
            continue;
        }
        if let Some(to) = c.span.1 {
            deltas.entry(to).or_default().remove.push(key.clone());
        }
        deltas
            .entry(c.span.0)
            .or_default()
            .add
            .push(Site::Call(CallSite {
                key: key.clone(),
                service: c.service,
                method: t.method.clone(),
                target: Target {
                    host: None,
                    segs: t.segs.clone(),
                    prefix_unknown: false,
                },
            }));
        infos.insert(
            (key, c.span.0),
            Info {
                file: c.file,
                kind: "generated",
                text: cut(&format!(
                    "{} -> {} {}",
                    c.name,
                    c.method.to_ascii_uppercase(),
                    c.path
                )),
                span: c.at,
                extractor: c.extractor,
                raw: c.path.clone(),
                template: format!("{} {}", t.method, t.path()),
                host: false,
                cfg: None,
            },
        );
    }
    // gRPC servers (XS-18): registering a generated service serves each rpc of its proto.
    for s in codegen::serve(&defs, &called, &views) {
        let t = normalise_route(&s.method, &s.path);
        let key = SiteKey {
            file: s.file,
            span: [s.at[0], s.at[1], s.op as u32, 0],
            env: 0,
            kind: Kind::Route,
        };
        if infos.contains_key(&(key.clone(), s.span.0)) {
            continue;
        }
        if let Some(to) = s.span.1 {
            deltas.entry(to).or_default().remove.push(key.clone());
        }
        deltas
            .entry(s.span.0)
            .or_default()
            .add
            .push(Site::Route(RouteSite {
                key: key.clone(),
                service: s.service,
                method: t.method.clone(),
                segs: t.segs.clone(),
            }));
        infos.insert(
            (key, s.span.0),
            Info {
                file: s.file,
                kind: "generated",
                text: cut(&format!("{} -> {} {}", s.name, s.method, s.path)),
                span: s.at,
                extractor: s.extractor,
                raw: s.path.clone(),
                template: t.to_string(),
                host: false,
                cfg: None,
            },
        );
    }
    // The operations: sites of the contract layer only.
    let mut op_deltas: BTreeMap<i64, Delta> = BTreeMap::new();
    let extractor = contracts::extractor();
    for o in ops {
        let t = normalise_route(&o.method, &o.path);
        let key = op_key(o);
        if let Some(to) = o.to {
            op_deltas.entry(to).or_default().remove.push(key.clone());
        }
        op_deltas
            .entry(o.from)
            .or_default()
            .add
            .push(Site::Route(RouteSite {
                key: key.clone(),
                service: o.service,
                method: t.method.clone(),
                segs: t.segs.clone(),
            }));
        infos.insert(
            (key, o.from),
            Info {
                file: o.file,
                kind: "contract",
                text: cut(&format!("{} {}", o.method.to_ascii_uppercase(), o.path)),
                span: [o.line, 0, o.line, 0],
                extractor: extractor.clone(),
                raw: o.path.clone(),
                template: t.to_string(),
                host: false,
                cfg: None,
            },
        );
    }
    (
        deltas,
        infos,
        node_of,
        op_deltas,
        read,
        unresolved,
        client_aliases,
        generated,
    )
}

/// Rebuilds the HTTP rows when the interface facts, the paths and versions of the files that
/// carry them or the host aliases changed. Returns the intervals written.
pub(crate) fn refresh(conn: &Connection, config: Option<&str>) -> rusqlite::Result<usize> {
    let rows = load(conn, false)?;
    let refs = load(conn, true)?;
    let aliases = load_aliases(conn)?;
    let ops = load_ops(conn)?;
    let contracts_key = contracts::built_key(conn)?;
    let envs = config::environments(conn)?;
    let values = load_values(conn)?;
    // The keys the sites read when the pass last ran: only their values are inputs.
    let stored: String = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM meta WHERE key = ?1",
            [READ_KEYS],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let inputs_key = |read: &BTreeSet<String>| {
        let mut inputs: Vec<Vec<u8>> = rows
            .iter()
            .map(|r| {
                let mut b = format!(
                    "site\0{}\0{}\0{}\0{:?}\0{}\0",
                    r.file, r.node, r.from, r.to, r.service
                )
                .into_bytes();
                b.extend_from_slice(&r.blob);
                b
            })
            .collect();
        inputs.extend(aliases.iter().map(|a| {
            format!(
                "alias\0{}\0{:?}\0{}\0{}\0{:?}",
                a.alias.host, a.alias.port, a.alias.service, a.from, a.to
            )
            .into_bytes()
        }));
        inputs.extend(refs.iter().map(|r| {
            let mut b = format!(
                "ref\0{}\0{}\0{}\0{:?}\0{}\0",
                r.file, r.node, r.from, r.to, r.service
            )
            .into_bytes();
            b.extend_from_slice(&r.blob);
            b
        }));
        inputs.push(codegen::refs_version().into_bytes());
        // The operations are rows of the contracts pass: its key says whether they changed.
        inputs.push([b"contracts\0".as_slice(), &contracts_key].concat());
        inputs.extend(
            envs.iter()
                .map(|e| format!("env\0{}\0{}", e.id, e.name).into_bytes()),
        );
        for k in read {
            for (src, group) in [(values.base.get(k), None), (None, Some(&values.own))] {
                let mut push = |env: Option<i64>, rows: &[Cv]| {
                    for (source, v, f, t) in rows {
                        inputs.push(
                            format!("cfg\0{k}\0{env:?}\0{source}\0{v:?}\0{f}\0{t:?}").into_bytes(),
                        );
                    }
                };
                if let Some(rows) = src {
                    push(None, rows);
                }
                if let Some(own) = group {
                    for e in &envs {
                        if let Some(rows) = own.get(&(k.clone(), e.id)) {
                            push(Some(e.id), rows);
                        }
                    }
                }
            }
        }
        // The adapters that made the facts, subprocess ones included (XS-24).
        inputs.push(
            conn.query_row(
                "SELECT value FROM meta WHERE key = 'adapters_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_default()
            .into_bytes(),
        );
        // A re-weight re-scores the links and nothing else.
        inputs.push(
            format!(
                "{}\0{}",
                confidence::EXTRACTOR,
                confidence::extractor_version()
            )
            .into_bytes(),
        );
        key("http", HTTP_VERSION, inputs, config)
    };
    let before: BTreeSet<String> = stored
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let k = inputs_key(&before);
    if !changed(conn, "http", "workspace", &k)? {
        return Ok(0);
    }

    let (deltas, mut infos, node_of, op_deltas, read, no_value, client_aliases, generated) =
        timeline(&rows, &refs, &aliases, &ops, &envs, &values);
    let mut sweep = Sweep {
        resolver: HttpResolver::new(),
        layer: (!ops.is_empty()).then(Layer::default),
        info: HashMap::new(),
        prev: HashMap::new(),
        node_of,
        counts: HashMap::new(),
        open_at: HashMap::new(),
        unres_open: HashMap::new(),
        cfg_counts: HashMap::new(),
        cfg_open: HashMap::new(),
        out: Out::default(),
    };
    let none = Delta::default();
    let seqs: BTreeSet<i64> = deltas.keys().chain(op_deltas.keys()).copied().collect();
    for seq in &seqs {
        sweep.prev.clear();
        let (d, o) = (
            deltas.get(seq).unwrap_or(&none),
            op_deltas.get(seq).unwrap_or(&none),
        );
        for s in d.add.iter().chain(&o.add) {
            let k = match s {
                Site::Call(c) => &c.key,
                Site::Route(r) => &r.key,
            };
            if let Some(i) = infos.remove(&(k.clone(), *seq))
                && let Some(old) = sweep.info.insert(k.clone(), i)
            {
                sweep.prev.insert(k.clone(), old);
            }
        }
        sweep.step(*seq, d, o);
    }
    let binds = sweep.layer.take().map(|l| l.rows).unwrap_or_default();
    let mut out = sweep.out;
    // One row per call and reason, whichever environment found it.
    out.unresolved.extend(no_value);
    out.unresolved.sort();
    out.unresolved.dedup();

    conn.execute_batch(
        "DELETE FROM link_config;
         DELETE FROM correction_interval; DELETE FROM correction_base;
         DELETE FROM evidence WHERE link IN (SELECT id FROM link WHERE kind = 'http');
         DELETE FROM interval WHERE link IN (SELECT id FROM link WHERE kind = 'http');
         DELETE FROM link WHERE kind = 'http';
         DELETE FROM unresolved WHERE fact_kind = 'http.call';
         DELETE FROM binding WHERE kind NOT IN ('publishes', 'subscribes');
         UPDATE node SET generated = NULL WHERE generated IS NOT NULL;",
    )?;
    codegen::write_aliases(conn, &client_aliases)?;
    for (node, generator) in &generated {
        conn.prepare_cached("UPDATE node SET generated = ?2 WHERE id = ?1")?
            .execute(params![node, cut(generator)])?;
    }
    for (b, from, to) in &binds {
        let Some(i) = sweep.info.get(&b.site) else {
            continue;
        };
        conn.prepare_cached(
            "INSERT INTO binding (kind, operation, file, line, template, from_seq, to_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            b.kind,
            b.op.as_ref().map(|k| i64::from(k.span[0])),
            i.file,
            i.span[0],
            i.template,
            from,
            to
        ])?;
    }
    let mut written = 0;
    let mut ids: HashMap<Link, i64> = HashMap::new();
    for (l, spans) in &out.intervals {
        let types: Vec<Ev> = out.types.get(l).into_iter().flatten().copied().collect();
        let c = confidence::score(&types);
        conn.prepare_cached(
            "INSERT INTO link (kind, src, dst, level, score, envs)
             VALUES ('http', ?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![
            l.0,
            l.1,
            c.level.as_str(),
            c.score,
            out.envs.get(l).map(|m| i64::from(*m))
        ])?;
        let id = conn.last_insert_rowid();
        ids.insert(*l, id);
        for (from, to) in spans {
            conn.prepare_cached(
                "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![id, from, to])?;
            written += 1;
        }
        for (file, kind, text, span, extractor, operation) in
            out.evidence.get(l).into_iter().flatten()
        {
            conn.prepare_cached(
                "INSERT INTO evidence (link, file, kind, text, start_line, start_col, end_line,
                   end_col, extractor, operation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                id,
                file,
                kind,
                cartograph_core::secrets::mask(text),
                span[0],
                span[1],
                span[2],
                span[3],
                extractor,
                operation
            ])?;
        }
    }
    for (file, template, line, reason, raw, from, to) in &out.unresolved {
        conn.prepare_cached(
            "INSERT INTO unresolved (file, spec, line, stage, reason, from_seq, to_seq, fact_kind, raw)
             VALUES (?1, ?2, ?3, 'resolve', ?4, ?5, ?6, 'http.call', ?7)",
        )?
        .execute(params![file, cut(template), line, reason, from, to, cut(raw)])?;
    }
    for ((l, env, key, value, source), from, to) in &out.cfg {
        let Some(link) = ids.get(l) else { continue };
        conn.prepare_cached(
            "INSERT INTO link_config (link, env_id, key, value, source, from_seq, to_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![link, env, key, value, source, from, to])?;
    }
    // The keys read now are the ones the next run hashes values for, so the key recorded is the
    // one it will compute.
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![
            READ_KEYS,
            read.iter().cloned().collect::<Vec<_>>().join("\n")
        ],
    )?;
    record(conn, "http", "workspace", &inputs_key(&read))?;
    Ok(written)
}
