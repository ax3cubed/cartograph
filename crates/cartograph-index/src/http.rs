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

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cartograph_facts::schema::{AttrValue, Attrs, EvidenceType, FactKind, Part};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, params};

use crate::derive::key;
use crate::ifaces;
use crate::links::confidence::{self, EvidenceType as Ev};
use crate::links::http::{
    Alias, CallSite, Delta, HttpResolver, Kind, LinkResolver, LinkSink, Match, Pair, Reason,
    RouteSite, Site, SiteKey, call_target, normalise_route,
};
use crate::rows::cut;

/// Bump when what the rows hold changes, so every index rebuilds them.
const HTTP_VERSION: &str = "http 2";

/// What a site says about itself, for evidence rows.
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
}

/// One version of a file at a path: its sites, over the overlap of the two intervals.
struct Row {
    file: i64,
    node: i64,
    service: i64,
    blob: Vec<u8>,
    from: i64,
    to: Option<i64>,
    payload: Vec<u8>,
}

fn load(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut st = conn.prepare_cached(
        "SELECT fp.file, n.id, coalesce(n.service, 0), fv.blob, fp.from_seq, fp.to_seq,
                fv.from_seq, fv.to_seq, fa.payload
         FROM fact fa JOIN extractor e ON e.id = fa.extractor AND e.name = ?1
           JOIN file_version fv ON fv.blob = fa.blob
           JOIN file_path fp ON fp.file = fv.file
           JOIN node n ON n.file = fv.file AND n.kind = 'file'
         ORDER BY fp.file, fv.blob, fp.from_seq, fv.from_seq",
    )?;
    let mut rows = st.query([ifaces::EXTRACTOR])?;
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

fn text_of(v: &AttrValue) -> Option<String> {
    match v {
        AttrValue::Str(s) => Some(s.as_str().to_owned()),
        AttrValue::Parts { .. } => None,
    }
}

/// `(is_expression, text)` pieces of a value, and the same as source-like text.
fn pieces(v: &AttrValue) -> Vec<(bool, String)> {
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

fn attr<'a>(a: &'a Attrs, k: &str) -> Option<&'a AttrValue> {
    a.get(k)
}

/// The sites of one version of a file.
fn sites(row: &Row, out: &mut Vec<(Site, Info)>) {
    let Some(per) = ifaces::decode(&row.payload) else {
        return;
    };
    for (name, version, facts) in per {
        for f in facts {
            let key = |kind| SiteKey {
                file: row.file,
                span: f.span.0,
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
                    out.push((
                        Site::Route(RouteSite {
                            key: key(Kind::Route),
                            service: row.service,
                            method: t.method,
                            segs: t.segs,
                        }),
                        info,
                    ));
                }
                FactKind::HttpCall => {
                    let Some(method) = attr(&f.attrs, "method").and_then(text_of) else {
                        continue;
                    };
                    let Some(url) = attr(&f.attrs, "url") else {
                        continue;
                    };
                    let ps = pieces(url);
                    let borrowed: Vec<(bool, &str)> =
                        ps.iter().map(|(e, t)| (*e, t.as_str())).collect();
                    let Some(target) = call_target(&borrowed) else {
                        continue;
                    };
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
                    info.host = target.host.is_some();
                    out.push((
                        Site::Call(CallSite {
                            key: key(Kind::Call),
                            service: row.service,
                            method,
                            target,
                        }),
                        info,
                    ));
                }
                _ => {}
            }
        }
    }
}

type Link = (i64, i64);
type EvidenceRow = (i64, &'static str, String, [u32; 4], String, String);
type UnresolvedRow = (i64, String, u32, &'static str, String, i64, Option<i64>);

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
}

struct Sweep {
    resolver: HttpResolver,
    info: HashMap<SiteKey, Info>,
    node_of: HashMap<i64, i64>,
    counts: HashMap<Link, i64>,
    open_at: HashMap<Link, usize>,
    unres_open: HashMap<SiteKey, (Reason, i64, usize)>,
    out: Out,
}

impl Sweep {
    fn step(&mut self, seq: i64, d: &Delta) {
        let mut s = Step::default();
        self.resolver.apply(seq as u64, d, &mut s);
        let link =
            |p: &Pair, n: &HashMap<i64, i64>| Some((*n.get(&p.call.file)?, *n.get(&p.route.file)?));
        let mut before: HashMap<Link, i64> = HashMap::new();
        for p in &s.close {
            if let Some(l) = link(p, &self.node_of).filter(|l| l.0 != l.1) {
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
            if let Some(i) = self.info.get(&p.call) {
                match i.kind {
                    "literal" => types.insert(Ev::LiteralCallSiteUrl),
                    "template" => types.insert(Ev::TemplateCallSiteUrl),
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
                r.as_str(),
                i.raw.clone(),
                seq,
                None,
            ));
            self.unres_open
                .insert(k.clone(), (*r, seq, self.out.unresolved.len() - 1));
        }
    }
}

/// The delta of each commit that changes a site or an alias.
type Timeline = (
    BTreeMap<i64, Delta>,
    HashMap<(SiteKey, i64), Info>,
    HashMap<i64, i64>,
);

fn timeline(rows: &[Row], aliases: &[Aliases]) -> Timeline {
    let mut deltas: BTreeMap<i64, Delta> = BTreeMap::new();
    // What each add says about itself, by (site, commit): a later version of the same place
    // replaces it in order.
    let mut infos = HashMap::new();
    let mut node_of = HashMap::new();
    for row in rows {
        node_of.insert(row.file, row.node);
        let mut v = Vec::new();
        sites(row, &mut v);
        for (site, info) in v {
            let k = match &site {
                Site::Call(c) => c.key.clone(),
                Site::Route(r) => r.key.clone(),
            };
            if let Some(to) = row.to {
                deltas.entry(to).or_default().remove.push(k.clone());
            }
            deltas.entry(row.from).or_default().add.push(site);
            infos.insert((k, row.from), info);
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
    (deltas, infos, node_of)
}

/// Rebuilds the HTTP rows when the interface facts, the paths and versions of the files that
/// carry them or the host aliases changed. Returns the intervals written.
pub(crate) fn refresh(conn: &Connection, config: Option<&str>) -> rusqlite::Result<usize> {
    let rows = load(conn)?;
    let aliases = load_aliases(conn)?;
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
    inputs.push(ifaces::version().into_bytes());
    // A re-weight re-scores the links and nothing else.
    inputs.push(
        format!(
            "{}\0{}",
            confidence::EXTRACTOR,
            confidence::extractor_version()
        )
        .into_bytes(),
    );
    let k = key("http", HTTP_VERSION, inputs, config);
    if !changed(conn, "http", "workspace", &k)? {
        return Ok(0);
    }

    let (deltas, mut infos, node_of) = timeline(&rows, &aliases);
    let mut sweep = Sweep {
        resolver: HttpResolver::new(),
        info: HashMap::new(),
        node_of,
        counts: HashMap::new(),
        open_at: HashMap::new(),
        unres_open: HashMap::new(),
        out: Out::default(),
    };
    for (seq, d) in &deltas {
        for s in &d.add {
            let k = match s {
                Site::Call(c) => &c.key,
                Site::Route(r) => &r.key,
            };
            if let Some(i) = infos.remove(&(k.clone(), *seq)) {
                sweep.info.insert(k.clone(), i);
            }
        }
        sweep.step(*seq, d);
    }
    let out = sweep.out;

    conn.execute_batch(
        "DELETE FROM evidence WHERE link IN (SELECT id FROM link WHERE kind = 'http');
         DELETE FROM interval WHERE link IN (SELECT id FROM link WHERE kind = 'http');
         DELETE FROM link WHERE kind = 'http';
         DELETE FROM unresolved WHERE fact_kind = 'http.call';",
    )?;
    let mut written = 0;
    for (l, spans) in &out.intervals {
        let types: Vec<Ev> = out.types.get(l).into_iter().flatten().copied().collect();
        let c = confidence::score(&types);
        conn.prepare_cached(
            "INSERT INTO link (kind, src, dst, level, score) VALUES ('http', ?1, ?2, ?3, ?4)",
        )?
        .execute(params![l.0, l.1, c.level.as_str(), c.score])?;
        let id = conn.last_insert_rowid();
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
    record(conn, "http", "workspace", &k)?;
    Ok(written)
}
