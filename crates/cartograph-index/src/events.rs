//! Event links in the index (XS-16): publishers and subscribers of one channel, linked.
//!
//! The adapters say, per blob, which channels a file publishes to (`event.publish`) and subscribes
//! to (`event.subscribe`). This pass reads them with the time each version of its file was at its
//! path, resolves the names (a channel through an environment variable is looked up in the config
//! values of each environment, XS-09), and matches every subscriber to the publishers it hears
//! (see `links::events`). It writes:
//!
//! - `channel_edge` rows: one per site and stretch, keyed by (broker, name). They are the reverse
//!   index from a channel to both sides, and the publishes and subscribes edges of the channel.
//! - an `event` link per pair of files, from the subscriber's file to the publisher's like every
//!   other kind (the override file reads `from` and `to` of an event key as publisher and
//!   consumer), with intervals, evidence and the level the confidence engine gives it.
//! - `binding` rows of kind `publishes` and `subscribes`, joining the operations of AsyncAPI specs
//!   (`contract.kind = 'events'`) to the sites that use their channel.
//!
//! Like the HTTP pass it is rebuilt whole when its inputs key changes, so a sync equals a rebuild.
//! An index with no event fact and no AsyncAPI spec pays one `meta` read and one `derived` read.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cartograph_facts::schema::FactKind;
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};

use crate::config::{self, Env};
use crate::derive::key;
use crate::http::{Row, Values, attr, env_var, load, load_values, name_of, pieces};
use crate::ifaces;
use crate::links::confidence::{self, EvidenceType as Ev};
use crate::links::events::{Name, address_matches, channel_name, is_pattern, matches, pair};
use crate::rows::cut;
use crate::structure::merge;

/// Bump when what the rows hold changes, so every index rebuilds them.
const EVENTS_VERSION: &str = "events 1";

type Span = (i64, Option<i64>);

/// (file, spec, line, reason, raw, from, to) of an `unresolved` row.
type Unresolved = (i64, String, u32, String, String, i64, Option<i64>);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Role {
    Publish,
    Subscribe,
}

impl Role {
    fn fact(self) -> &'static str {
        match self {
            Role::Publish => "event.publish",
            Role::Subscribe => "event.subscribe",
        }
    }
}

/// One place a file publishes or subscribes, over one stretch of its history.
struct Site {
    role: Role,
    broker: String,
    name: Name,
    file: i64,
    node: i64,
    line: u32,
    span: [u32; 4],
    text: String,
    literal: bool,
    extractor: String,
    /// The environment the name was resolved in, if it came from a value.
    env: Option<i64>,
    from: i64,
    to: Option<i64>,
}

impl Site {
    /// The channel the site names, when it names one.
    fn channel(&self) -> Option<&str> {
        match &self.name {
            Name::Literal(n) | Name::Config(n) => Some(n),
            _ => None,
        }
    }
}

/// A call or a spec operation of an AsyncAPI document, over the time its version had it.
struct Op {
    id: i64,
    address: String,
    from: i64,
    to: Option<i64>,
}

fn load_ops(conn: &Connection) -> rusqlite::Result<Vec<Op>> {
    conn.prepare_cached(
        "SELECT o.id, o.path, o.from_seq, o.to_seq FROM operation o
         JOIN contract c ON c.id = o.contract WHERE c.kind = 'events' ORDER BY o.id",
    )?
    .query_map([], |r| {
        Ok(Op {
            id: r.get(0)?,
            address: r.get(1)?,
            from: r.get(2)?,
            to: r.get(3)?,
        })
    })?
    .collect()
}

fn overlap(a: Span, b: Span) -> Option<Span> {
    let from = a.0.max(b.0);
    let to = match (a.1, b.1) {
        (None, t) | (t, None) => t,
        (Some(x), Some(y)) => Some(x.min(y)),
    };
    to.is_none_or(|t| from < t).then_some((from, to))
}

/// What an expression that isn't a string stands for.
enum Pending {
    /// A name read from the environment variable.
    Env(String),
    Done(Name),
}

fn classify(parts: &[(bool, String)], bound: &HashMap<String, String>) -> Option<Pending> {
    match parts {
        [] => None,
        [(false, s)] => Some(Pending::Done(Name::Literal(channel_name(s)))),
        [(true, e)] => Some(match env_var(e, bound) {
            Some(v) => Pending::Env(v),
            None => Pending::Done(Name::Constant(e.trim().trim_end_matches('!').to_owned())),
        }),
        // `orders.${region}`: what the code spells before its first unknown.
        [(false, s), ..] => Some(Pending::Done(Name::Prefix(s.clone()))),
        _ => None,
    }
}

/// The sites of one version of a file, and the env variables they wait on.
fn sites(
    row: &Row,
    envs: &[Env],
    values: &Values,
    read: &mut BTreeSet<String>,
    out: &mut Vec<Site>,
    unresolved: &mut Vec<Unresolved>,
) {
    let Some(per) = ifaces::decode(&row.payload) else {
        return;
    };
    let mut bound: HashMap<String, String> = HashMap::new();
    for (_, _, facts) in &per {
        for f in facts {
            if f.kind == FactKind::EnvRead
                && let (Some(k), Some(n)) = (
                    attr(&f.attrs, "key").and_then(name_of),
                    attr(&f.attrs, "name").and_then(name_of),
                )
            {
                bound.insert(n, k);
            }
        }
    }
    for (adapter, version, facts) in per {
        for f in facts {
            let role = match f.kind {
                FactKind::EventPublish => Role::Publish,
                FactKind::EventSubscribe => Role::Subscribe,
                _ => continue,
            };
            let (Some(broker), Some(topic)) = (
                attr(&f.attrs, "broker").and_then(name_of),
                attr(&f.attrs, "topic"),
            ) else {
                continue;
            };
            let Some(pending) = classify(&pieces(topic), &bound) else {
                continue;
            };
            let make = |name: Name, env: Option<i64>, from: i64, to: Option<i64>| Site {
                role,
                broker: broker.clone(),
                name,
                file: row.file,
                node: row.node,
                line: f.span.0[0],
                span: f.span.0,
                // Masked again: this is the stored copy. A queue URL is cut to its name.
                text: if matches!(broker.as_str(), "sqs" | "sns") {
                    cut(&format!("{} {broker}", role.fact()))
                } else {
                    cut(f.evidence.text.as_str())
                },
                literal: matches!(
                    f.evidence.kind,
                    cartograph_facts::schema::EvidenceType::Literal
                ),
                extractor: format!("{adapter}@{version}"),
                env,
                from,
                to,
            };
            match pending {
                Pending::Done(n) => out.push(make(n, None, row.from, row.to)),
                Pending::Env(var) => {
                    read.insert(var.clone());
                    let none = Vec::new();
                    let base = values.base.get(&var).unwrap_or(&none);
                    for env in envs {
                        let own = values.own.get(&(var.clone(), env.id)).unwrap_or(&none);
                        for (from, to, found) in config::effective(own, base, (row.from, row.to)) {
                            match found {
                                Some((value, _)) => out.push(make(
                                    Name::Config(channel_name(&value)),
                                    Some(env.id),
                                    from,
                                    to,
                                )),
                                None => unresolved.push((
                                    row.file,
                                    format!("{} {broker}", role.fact()),
                                    f.span.0[0],
                                    format!("no value for {var} in {}", env.name),
                                    cut(&var),
                                    from,
                                    to,
                                )),
                            }
                        }
                    }
                    if envs.is_empty() {
                        unresolved.push((
                            row.file,
                            format!("{} {broker}", role.fact()),
                            f.span.0[0],
                            format!("no value for {var}"),
                            cut(&var),
                            row.from,
                            row.to,
                        ));
                    }
                }
            }
        }
    }
}

/// Whether anything can have changed since a pass that found nothing: no event fact is in the
/// index and no AsyncAPI spec is.
fn nothing_to_do(conn: &Connection) -> rusqlite::Result<bool> {
    let blobs: Option<i64> = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = ?1",
            [ifaces::EVENT_BLOBS],
            |r| r.get(0),
        )
        .optional()?;
    let blobs = match blobs {
        Some(n) => n,
        None => {
            // An index made before the count existed: counted once.
            let n: i64 = conn.query_row(
                "SELECT count(*) FROM fact fa JOIN extractor e ON e.id = fa.extractor AND e.name = ?1
                 WHERE instr(fa.payload, CAST('\"event.' AS BLOB)) > 0",
                [ifaces::EXTRACTOR],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                params![ifaces::EVENT_BLOBS, n],
            )?;
            n
        }
    };
    let specs: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM contract WHERE kind = 'events')",
        [],
        |r| r.get(0),
    )?;
    Ok(blobs == 0 && !specs)
}

fn clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DELETE FROM evidence WHERE link IN (SELECT id FROM link WHERE kind = 'event');
         DELETE FROM interval WHERE link IN (SELECT id FROM link WHERE kind = 'event');
         DELETE FROM link WHERE kind = 'event';
         DELETE FROM channel_edge;
         DELETE FROM binding WHERE kind IN ('publishes', 'subscribes');
         DELETE FROM unresolved WHERE fact_kind IN ('event.publish', 'event.subscribe');",
    )
}

/// Rebuilds the event rows when the event facts, the paths and versions of the files that carry
/// them, the AsyncAPI operations or the config values they read changed. Returns the intervals
/// written.
pub(crate) fn refresh(conn: &Connection, config: Option<&str>) -> rusqlite::Result<usize> {
    let empty = nothing_to_do(conn)?;
    let rows: Vec<Row> = if empty {
        Vec::new()
    } else {
        load(conn, false)?
            .into_iter()
            .filter(|r| r.payload.windows(7).any(|w| w == b"\"event."))
            .collect()
    };
    let ops = if empty { Vec::new() } else { load_ops(conn)? };
    let envs = config::environments(conn)?;
    let values = if rows.is_empty() {
        Values::default()
    } else {
        load_values(conn)?
    };
    let mut read = BTreeSet::new();
    let mut all: Vec<Site> = Vec::new();
    let mut no_value = Vec::new();
    for r in &rows {
        sites(r, &envs, &values, &mut read, &mut all, &mut no_value);
    }

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
    inputs.extend(
        ops.iter()
            .map(|o| format!("op\0{}\0{}\0{}\0{:?}", o.id, o.address, o.from, o.to).into_bytes()),
    );
    inputs.extend(
        envs.iter()
            .map(|e| format!("env\0{}\0{}", e.id, e.name).into_bytes()),
    );
    for k in &read {
        for (env, rows) in values.base.get(k).map(|r| (None, r)).into_iter().chain(
            envs.iter()
                .filter_map(|e| values.own.get(&(k.clone(), e.id)).map(|r| (Some(e.id), r))),
        ) {
            for (source, v, f, t) in rows {
                inputs.push(format!("cfg\0{k}\0{env:?}\0{source}\0{v:?}\0{f}\0{t:?}").into_bytes());
            }
        }
    }
    inputs.push(ifaces::version().into_bytes());
    inputs.push(
        format!(
            "{}\0{}",
            confidence::EXTRACTOR,
            confidence::extractor_version()
        )
        .into_bytes(),
    );
    let k = key("events", EVENTS_VERSION, inputs, config);
    if !changed(conn, "events", "workspace", &k)? {
        return Ok(0);
    }
    clear(conn)?;
    if all.is_empty() && ops.is_empty() && no_value.is_empty() {
        record(conn, "events", "workspace", &k)?;
        return Ok(0);
    }

    // The reverse index: the publishers' sites of each (broker, channel), and the subscribers'.
    let mut by_name: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    let mut prefixes: Vec<usize> = Vec::new();
    let mut consts: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (i, s) in all
        .iter()
        .enumerate()
        .filter(|(_, s)| s.role == Role::Publish)
    {
        match &s.name {
            Name::Literal(n) | Name::Config(n) => by_name
                .entry((s.broker.as_str(), n.as_str()))
                .or_default()
                .push(i),
            Name::Prefix(_) => prefixes.push(i),
            Name::Constant(c) => consts
                .entry((s.broker.as_str(), c.as_str()))
                .or_default()
                .push(i),
        }
    }
    // For each subscriber, the publishers it hears.
    let mut pairs: BTreeSet<(usize, usize, Ev)> = BTreeSet::new();
    for (si, s) in all
        .iter()
        .enumerate()
        .filter(|(_, s)| s.role == Role::Subscribe)
    {
        let b = s.broker.as_str();
        let mut hear = |pi: usize| {
            let p = &all[pi];
            if p.broker == s.broker
                && p.node != s.node
                && (p.env.is_none() || s.env.is_none() || p.env == s.env)
                && let Some(ev) = pair(b, &p.name, &s.name)
            {
                pairs.insert((pi, si, ev));
            }
        };
        match &s.name {
            Name::Literal(n) | Name::Config(n) if is_pattern(b, n) => {
                for ((pb, pn), v) in &by_name {
                    if *pb == b && matches(b, n, pn) {
                        v.iter().for_each(|&pi| hear(pi));
                    }
                }
            }
            Name::Literal(n) | Name::Config(n) => {
                if let Some(v) = by_name.get(&(b, n.as_str())) {
                    v.iter().for_each(|&pi| hear(pi));
                }
            }
            Name::Prefix(p) => {
                for ((pb, pn), v) in by_name.range((b, p.as_str())..) {
                    if *pb != b || !pn.starts_with(p.as_str()) {
                        break;
                    }
                    v.iter().for_each(|&pi| hear(pi));
                }
            }
            Name::Constant(c) => {
                if let Some(v) = consts.get(&(b, c.as_str())) {
                    v.iter().for_each(|&pi| hear(pi));
                }
            }
        }
        if let Some(n) = s.channel() {
            for &pi in &prefixes {
                if matches!(&all[pi].name, Name::Prefix(p) if n.starts_with(p.as_str())) {
                    hear(pi);
                }
            }
        }
    }

    // Links: subscriber's file to publisher's, over the stretches both sites held.
    type Link = (i64, i64);
    let mut spans: BTreeMap<Link, Vec<Span>> = BTreeMap::new();
    let mut types: BTreeMap<Link, BTreeSet<Ev>> = BTreeMap::new();
    let mut envmask: BTreeMap<Link, u32> = BTreeMap::new();
    let mut evidence: BTreeMap<Link, BTreeSet<usize>> = BTreeMap::new();
    for (pi, si, ev) in &pairs {
        let (p, s) = (&all[*pi], &all[*si]);
        let Some(at) = overlap((p.from, p.to), (s.from, s.to)) else {
            continue;
        };
        let l = (s.node, p.node);
        spans.entry(l).or_default().push(at);
        types.entry(l).or_default().insert(*ev);
        if let Some(e) = p.env.or(s.env) {
            *envmask.entry(l).or_default() |= 1u32 << (e as u32 % 32);
        }
        evidence.entry(l).or_default().extend([*pi, *si]);
    }
    let mut written = 0;
    for (l, sp) in spans {
        let c = confidence::score(&types[&l].iter().copied().collect::<Vec<_>>());
        conn.prepare_cached(
            "INSERT INTO link (kind, src, dst, level, score, envs)
             VALUES ('event', ?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![
            l.0,
            l.1,
            c.level.as_str(),
            c.score,
            envmask.get(&l)
        ])?;
        let id = conn.last_insert_rowid();
        for (from, to) in merge(sp) {
            conn.prepare_cached(
                "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![id, from, to])?;
            written += 1;
        }
        let mut seen = HashSet::new();
        for &i in &evidence[&l] {
            let s = &all[i];
            if !seen.insert((s.file, s.span, s.role)) {
                continue;
            }
            conn.prepare_cached(
                "INSERT INTO evidence (link, file, kind, text, start_line, start_col, end_line,
                   end_col, extractor, operation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            )?
            .execute(params![
                id,
                s.file,
                if s.literal { "literal" } else { "template" },
                cartograph_core::secrets::mask(&s.text),
                s.span[0],
                s.span[1],
                s.span[2],
                s.span[3],
                s.extractor
            ])?;
        }
    }

    // The channels' edges: one row per file, line, role and channel over the time it held.
    let mut edges: BTreeMap<(&str, &str, Role, i64, u32), Vec<Span>> = BTreeMap::new();
    for s in &all {
        if let Some(n) = s.channel() {
            edges
                .entry((s.broker.as_str(), n, s.role, s.file, s.line))
                .or_default()
                .push((s.from, s.to));
        }
    }
    for ((broker, name, role, file, line), sp) in edges {
        for (from, to) in merge(sp) {
            conn.prepare_cached(
                "INSERT INTO channel_edge (broker, name, role, file, line, from_seq, to_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                broker,
                cut(name),
                if role == Role::Publish {
                    "publish"
                } else {
                    "subscribe"
                },
                file,
                line,
                from,
                to
            ])?;
        }
    }

    // Spec operations: the sites that use the channel they name.
    let mut binds: BTreeMap<(i64, Role, i64, u32, String), Vec<Span>> = BTreeMap::new();
    for o in &ops {
        for s in &all {
            let Some(n) = s.channel() else { continue };
            if address_matches(&o.address, n)
                && let Some(at) = overlap((o.from, o.to), (s.from, s.to))
            {
                binds
                    .entry((o.id, s.role, s.file, s.line, n.to_owned()))
                    .or_default()
                    .push(at);
            }
        }
    }
    for ((op, role, file, line, name), sp) in binds {
        for (from, to) in merge(sp) {
            conn.prepare_cached(
                "INSERT INTO binding (kind, operation, file, line, template, from_seq, to_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                if role == Role::Publish {
                    "publishes"
                } else {
                    "subscribes"
                },
                op,
                file,
                line,
                cut(&name),
                from,
                to
            ])?;
        }
    }

    no_value.sort();
    no_value.dedup();
    for (file, spec, line, reason, raw, from, to) in &no_value {
        let fact = if spec.starts_with(Role::Publish.fact()) {
            Role::Publish.fact()
        } else {
            Role::Subscribe.fact()
        };
        conn.prepare_cached(
            "INSERT INTO unresolved (file, spec, line, stage, reason, from_seq, to_seq, fact_kind, raw)
             VALUES (?1, ?2, ?3, 'resolve', ?4, ?5, ?6, ?7, ?8)",
        )?
        .execute(params![file, cut(spec), line, reason, from, to, fact, raw])?;
    }
    record(conn, "events", "workspace", &k)?;
    Ok(written)
}
