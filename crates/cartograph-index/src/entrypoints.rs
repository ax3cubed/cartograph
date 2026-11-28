//! Entrypoints in the index (XS-20): what starts work in a service, of six kinds.
//!
//! - `http`: every route a query adapter found, as the HTTP pass reads it (`get /orders/{}`).
//! - `event`: a file that subscribes to a channel (the `subscribe` rows of `channel_edge`).
//! - `job`: a k8s CronJob, or a scheduler a source file registers.
//! - `cli`: a package's `bin` and a Go `main` package.
//! - `export`: the files a package's `main`, `module` and `exports` name.
//! - `ui`: a page of a file-system router.
//!
//! The rows are `entrypoint` (one per stretch of history, named by a `uid` that survives a rebuild)
//! and `entrypoint_client` (the manifest's `clients:` that call an `http` one). Who calls an
//! entrypoint, and what it reaches, are read from the links at query time; the only reach stored is
//! the count at HEAD (`entrypoint.reach`), so a list needs no walk.
//!
//! Both passes are rebuilt whole when their inputs key changes, so a sync equals a rebuild. The
//! reach counts are recomputed for every entrypoint when the key of the links at HEAD changes (the
//! facts, the passes that write links, the corrections, the paths) and not otherwise: with one CSR
//! built per recompute, walking 431 entrypoints costs about what the scan of the links costs.

use std::collections::{BTreeMap, HashMap};

use cartograph_facts::adapters::AdapterSet;
use cartograph_facts::schema::FactKind;
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{Connection, Result, params};

use crate::derive::key;
use crate::http::{Prep, Row, attr, load, sites, text_of};
use crate::ifaces;
use crate::links::http::{Site, normalise_route};
use crate::rows::cut;
use crate::structure::merge;

/// Bump when what the rows hold changes, so every index rebuilds them.
const VERSION: &str = "entrypoints 1";
/// Bump when what a reach count means changes.
const REACH_VERSION: &str = "reach 1";

type Span = (i64, Option<i64>);

/// One `clients:` entry of the manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientSpec {
    pub name: String,
    pub service: Option<String>,
    pub calls: Vec<String>,
}

impl From<&cartograph_discovery::manifest::Client> for ClientSpec {
    fn from(c: &cartograph_discovery::manifest::Client) -> Self {
        Self {
            name: c.name.clone(),
            service: c.service.clone(),
            calls: c.calls.clone(),
        }
    }
}

/// `(kind, node, key)`: an entrypoint, whatever stretches it had.
type Id = (&'static str, i64, String);

#[derive(Default)]
struct Found {
    name: String,
    service: i64,
    spans: Vec<Span>,
}

/// The same hash for the same entrypoint on every run and machine.
fn uid(kind: &str, node: i64, key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{kind}\0{node}\0{key}").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("e{h:016x}")
}

fn kind_of(s: &str) -> Option<&'static str> {
    ["job", "cli", "export", "ui"].into_iter().find(|k| *k == s)
}

/// `dir/./a/../b` as `dir/b`.
fn join(dir: &str, rel: &str) -> String {
    let mut out: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// The file nodes at `path` in the repo of `file`, with the steps each held it.
fn at_path(conn: &Connection, file: i64, path: &str) -> Result<Vec<(i64, Span)>> {
    conn.prepare_cached(
        "SELECT n.id, fp.from_seq, fp.to_seq FROM file_path fp
         JOIN node n ON n.file = fp.file AND n.kind = 'file'
         WHERE fp.path = ?1 AND (SELECT repo FROM file WHERE id = fp.file)
                              = (SELECT repo FROM file WHERE id = ?2)",
    )?
    .query_map(params![path, file], |r| {
        Ok((r.get(0)?, (r.get(1)?, r.get(2)?)))
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

/// The entrypoints the facts of one version of a file say, over the stretch it held.
fn from_facts(conn: &Connection, row: &Row, out: &mut BTreeMap<Id, Found>) -> Result<()> {
    let mut add = |kind, node, key: String, name: String, spans: Vec<Span>| {
        let f = out.entry((kind, node, key)).or_default();
        f.name = name;
        f.service = row.service;
        f.spans.extend(spans);
    };
    let span = vec![(row.from, row.to)];
    let mut preps = Vec::new();
    sites(row, &mut preps);
    for p in preps {
        if let Prep::Site(Site::Route(_), info) = p {
            let (m, path) = info
                .template
                .split_once(' ')
                .unwrap_or(("", &info.template));
            add(
                "http",
                row.node,
                info.template.clone(),
                format!("{} {path}", m.to_ascii_uppercase()),
                span.clone(),
            );
        }
    }
    let Some(per) = ifaces::decode(&row.payload) else {
        return Ok(());
    };
    for (adapter, _, facts) in per {
        for f in facts.iter().filter(|f| f.kind == FactKind::Entrypoint) {
            let text = |k| attr(&f.attrs, k).and_then(text_of);
            let Some(kind) = text("kind").as_deref().and_then(kind_of) else {
                continue;
            };
            match kind {
                "ui" => {
                    let path = AdapterSet::builtin()
                        .adapters()
                        .iter()
                        .find(|a| a.name == adapter)
                        .and_then(|a| a.path_for("entrypoint", &row.path));
                    if let Some(p) = path {
                        add(kind, row.node, p.clone(), p, span.clone());
                    }
                }
                "export" | "cli" if text("target").is_some() => {
                    let dir = row.path.rsplit_once('/').map_or("", |x| x.0);
                    let target = join(dir, &text("target").unwrap_or_default());
                    let name = text("name").unwrap_or_default();
                    let hit: Vec<_> = at_path(conn, row.file, &target)?
                        .into_iter()
                        .filter_map(|(n, s)| overlap(s, (row.from, row.to)).map(|o| (n, o)))
                        .collect();
                    let key = format!("{name}\0{target}");
                    // A target that is no file in the index (a build output) stands for the
                    // manifest that names it.
                    if hit.is_empty() {
                        add(kind, row.node, key, name, span.clone());
                    } else {
                        for (n, o) in hit {
                            add(kind, n, key.clone(), name.clone(), vec![o]);
                        }
                    }
                }
                _ => {
                    let Some(name) = text("name") else { continue };
                    let label = match text("schedule") {
                        Some(s) => format!("{name} ({s})"),
                        None => name.clone(),
                    };
                    add(kind, row.node, name, label, span.clone());
                }
            }
        }
    }
    Ok(())
}

/// The services' names by node id, for `clients:` entries that name one.
fn service_names(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    let mut m: HashMap<i64, Vec<String>> = HashMap::new();
    let mut st =
        conn.prepare("SELECT id, name, key FROM node WHERE kind IN ('service', 'library')")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let v = m.entry(r.get(0)?).or_default();
        v.extend(r.get::<_, Option<String>>(1)?);
        v.extend(r.get::<_, Option<String>>(2)?);
    }
    Ok(m)
}

/// `GET /orders/{id}` as the key of a route (`get /orders/{}`).
fn call_key(call: &str) -> Option<String> {
    let (m, p) = call.trim().split_once(char::is_whitespace)?;
    Some(normalise_route(m, p.trim()).to_string())
}

/// Rebuilds the entrypoint rows when the facts they come from, the paths they resolve through,
/// the channels they subscribe to or the declared clients changed. Returns the rows written.
pub(crate) fn refresh(
    conn: &Connection,
    clients: &[ClientSpec],
    config: Option<&str>,
    structure_keys: &[Vec<u8>],
) -> Result<usize> {
    let rows: Vec<Row> = load(conn, false)?
        .into_iter()
        .filter(|r| {
            let has = |n: &[u8]| r.payload.windows(n.len()).any(|w| w == n);
            has(b"\"route\"") || has(b"\"entrypoint\"")
        })
        .collect();
    let subs: Vec<(String, String, i64, i64, i64, Option<i64>)> = conn
        .prepare_cached(
            "SELECT ce.broker, ce.name, n.id, coalesce(n.service, 0), ce.from_seq, ce.to_seq
             FROM channel_edge ce JOIN node n ON n.file = ce.file AND n.kind = 'file'
             WHERE ce.role = 'subscribe' ORDER BY ce.broker, ce.name, n.id, ce.from_seq",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<_>>()?;
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
    inputs.extend(subs.iter().map(|s| format!("sub\0{s:?}").into_bytes()));
    inputs.extend(
        clients
            .iter()
            .map(|c| format!("client\0{c:?}").into_bytes()),
    );
    // A package's targets resolve through paths: the structure keys carry their fingerprint.
    inputs.extend(structure_keys.iter().cloned());
    inputs.push(
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'adapters_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default()
        .into_bytes(),
    );
    let k = key("entrypoints", VERSION, inputs, config);
    if !changed(conn, "entrypoints", "workspace", &k)? {
        return Ok(0);
    }

    let mut found: BTreeMap<Id, Found> = BTreeMap::new();
    for r in &rows {
        from_facts(conn, r, &mut found)?;
    }
    for (broker, name, node, service, from, to) in &subs {
        let f = found
            .entry(("event", *node, format!("{broker}\0{name}")))
            .or_default();
        f.name = format!("on {name}");
        f.service = *service;
        f.spans.push((*from, *to));
    }

    conn.execute_batch("DELETE FROM entrypoint; DELETE FROM entrypoint_client;")?;
    let names = service_names(conn)?;
    let mut written = 0;
    for ((kind, node, key), f) in found {
        let id = uid(kind, node, &key);
        for (from, to) in merge(f.spans) {
            conn.prepare_cached(
                "INSERT INTO entrypoint (uid, kind, node, service, name, key, from_seq, to_seq)
                 VALUES (?1, ?2, ?3, nullif(?4, 0), ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                id,
                kind,
                node,
                f.service,
                cut(&f.name),
                cut(&key),
                from,
                to
            ])?;
            written += 1;
        }
        if kind == "http" {
            for c in clients {
                let named = c
                    .service
                    .as_ref()
                    .is_none_or(|s| names.get(&f.service).is_some_and(|n| n.contains(s)));
                if named && c.calls.iter().any(|x| call_key(x).as_deref() == Some(&key)) {
                    conn.prepare_cached(
                        "INSERT INTO entrypoint_client (uid, name) VALUES (?1, ?2)",
                    )?
                    .execute(params![id, cut(&c.name)])?;
                }
            }
        }
    }
    record(conn, "entrypoints", "workspace", &k)?;
    Ok(written)
}

/// Counts what each entrypoint open at HEAD reaches, when anything that decides it changed:
/// `facts` is the run's fact hash, and the derived keys of the passes that write links, of the
/// corrections and of the entrypoints stand for the rest.
pub(crate) fn reach(
    conn: &Connection,
    config: Option<&str>,
    facts: &[u8],
    structure_keys: &[Vec<u8>],
) -> Result<usize> {
    let mut inputs = vec![facts.to_vec()];
    inputs.extend(structure_keys.iter().cloned());
    let mut st = conn.prepare_cached(
        "SELECT kind, scope, inputs_key FROM derived
         WHERE kind IN ('http', 'events', 'tables', 'corrections', 'entrypoints', 'contracts')
         ORDER BY kind, scope",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let mut b =
            format!("{}\0{}\0", r.get::<_, String>(0)?, r.get::<_, String>(1)?).into_bytes();
        b.extend(r.get::<_, Vec<u8>>(2)?);
        inputs.push(b);
    }
    let k = key("reach", REACH_VERSION, inputs, config);
    if !changed(conn, "reach", "workspace", &k)? {
        return Ok(0);
    }
    let open: Vec<(i64, i64)> = conn
        .prepare_cached("SELECT id, node FROM entrypoint WHERE to_seq IS NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_>>()?;
    conn.execute(
        "UPDATE entrypoint SET reach = NULL WHERE to_seq IS NOT NULL AND reach IS NOT NULL",
        [],
    )?;
    if !open.is_empty() {
        let links: Vec<(i64, i64)> = conn
            .prepare_cached(
                "SELECT l.src, l.dst FROM interval i JOIN link l ON l.id = i.link
                 WHERE i.to_seq IS NULL",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_>>()?;
        let mut ids: Vec<i64> = links.iter().flat_map(|l| [l.0, l.1]).collect();
        ids.extend(open.iter().map(|o| o.1));
        ids.sort_unstable();
        ids.dedup();
        let dense = |id: i64| ids.binary_search(&id).expect("every end is listed") as u32;
        let mut off = vec![0u32; ids.len() + 1];
        for l in &links {
            off[dense(l.0) as usize + 1] += 1;
        }
        for i in 0..ids.len() {
            off[i + 1] += off[i];
        }
        let mut fill = off.clone();
        let mut to = vec![0u32; links.len()];
        for l in &links {
            let at = &mut fill[dense(l.0) as usize];
            to[*at as usize] = dense(l.1);
            *at += 1;
        }
        for (id, node) in open {
            let r = cartograph_core::reach::bfs(ids.len(), dense(node), |n, push| {
                to[off[n as usize] as usize..off[n as usize + 1] as usize]
                    .iter()
                    .for_each(|&m| push(m));
            });
            conn.prepare_cached("UPDATE entrypoint SET reach = ?2 WHERE id = ?1")?
                .execute(params![id, r.nodes.len() as i64])?;
        }
    }
    record(conn, "reach", "workspace", &k)?;
    Ok(1)
}
