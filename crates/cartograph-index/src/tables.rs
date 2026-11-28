//! Table links in the index (XS-17): files linked to the tables their SQL reads and writes, across
//! services. Runs after the HTTP pass, from the table facts of `crate::ifaces` (the extractor
//! `tables`) and the files' services.
//!
//! A table is a node (`table:<service key>:<name>`) owned by the service whose files define it. A
//! use is a `link` of kind `db` from the file that reads or writes to the table's node, live over
//! the steps in which both the file's version and a definition of the table are. The level comes
//! from how the name matched (see `crate::links::tables`). The pass is rebuilt whole when its
//! inputs key changes, and a sync equals a rebuild by construction. Nothing here opens a database.

use std::collections::{BTreeMap, BTreeSet};

use cartograph_facts::schema::{self, AttrValue, BlobFact, EvidenceType, FactKind};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{Connection, Result, params};

use crate::derive::key;
use crate::http::load_from;
use crate::ifaces::TABLES;
use crate::links::confidence::{self, EvidenceType as Ev};
use crate::links::tables::{DefKey, Defs, Match, VERSION, intersect, merge};
use crate::rows::cut;

/// Bump when what the rows hold changes, so every index rebuilds them.
const PASS_VERSION: &str = "tables-links 1";

/// What an evidence row says: (file, kind, text, span, operation).
type Site = (i64, &'static str, String, [u32; 4], String);

/// Where a fact stands.
fn site(file: i64, f: &BlobFact, operation: String) -> Site {
    let kind = match f.evidence.kind {
        EvidenceType::Literal => "literal",
        EvidenceType::Template => "template",
        EvidenceType::Inferred => "inferred",
    };
    (
        file,
        kind,
        f.evidence.text.as_str().to_owned(),
        f.span.0,
        operation,
    )
}

fn attr<'a>(f: &'a BlobFact, k: &str) -> Option<&'a str> {
    match f.attrs.get(k)? {
        AttrValue::Str(s) => Some(s.as_str()),
        AttrValue::Parts { .. } => None,
    }
}

/// One read or write of a table by a file.
struct Use<'a> {
    node: i64,
    service: i64,
    from: i64,
    to: Option<i64>,
    write: bool,
    file: i64,
    fact: &'a BlobFact,
}

/// What one link is made of.
#[derive(Default)]
struct Acc {
    spans: Vec<(i64, Option<i64>)>,
    types: BTreeSet<Ev>,
    sites: BTreeSet<Site>,
}

pub(crate) fn refresh(conn: &Connection, config: Option<&str>) -> Result<usize> {
    let rows = load_from(conn, TABLES, false)?;
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
    inputs.push(VERSION.as_bytes().to_vec());
    // A re-weight re-scores the links and nothing else.
    inputs.push(confidence::extractor_version().as_bytes().to_vec());
    let k = key("tables", PASS_VERSION, inputs, config);
    if !changed(conn, "tables", "workspace", &k)? {
        return Ok(0);
    }

    let facts: Vec<Vec<BlobFact>> = rows
        .iter()
        .map(|r| schema::decode(&r.payload).unwrap_or_default())
        .collect();
    // A table's definitions: migrations that create it and models for it, each in a service.
    let mut defs = Defs::default();
    let mut def_at: BTreeMap<DefKey, Site> = BTreeMap::new();
    let mut uses: Vec<Use> = Vec::new();
    for (r, fs) in rows.iter().zip(&facts) {
        for f in fs {
            let Some(name) = attr(f, "name") else {
                continue;
            };
            match f.kind {
                FactKind::DbTableDef
                    if r.service != 0 && matches!(attr(f, "op"), None | Some("create")) =>
                {
                    let convention = attr(f, "how") == Some("convention");
                    defs.add(name, r.service, convention, (r.from, r.to));
                    def_at
                        .entry((name.to_owned(), r.service, convention))
                        .or_insert_with(|| site(r.file, f, String::new()));
                }
                FactKind::DbRead | FactKind::DbWrite => uses.push(Use {
                    node: r.node,
                    service: r.service,
                    from: r.from,
                    to: r.to,
                    write: f.kind == FactKind::DbWrite,
                    file: r.file,
                    fact: f,
                }),
                _ => {}
            }
        }
    }
    defs.finish();

    let mut links: BTreeMap<(i64, DefKey), Acc> = BTreeMap::new();
    for u in &uses {
        let name = attr(u.fact, "name").unwrap_or_default();
        let Some((def, how)) = defs.resolve(name, u.service) else {
            continue;
        };
        let live = intersect(&[(u.from, u.to)], defs.spans(def));
        if live.is_empty() {
            continue;
        }
        let op = format!("{} {}", if u.write { "write" } else { "read" }, def.0);
        let acc = links.entry((u.node, def.clone())).or_default();
        acc.spans.extend(live);
        acc.types.insert(match how {
            Match::Defined => Ev::TableDefined,
            Match::Convention => Ev::TableConvention,
            Match::NameOnly => Ev::TableNameOnly,
        });
        acc.sites.insert(site(u.file, u.fact, op.clone()));
        if let Some(d) = def_at.get(def) {
            acc.sites.insert((d.0, d.1, d.2.clone(), d.3, op));
        }
    }

    conn.execute_batch(
        "DELETE FROM evidence WHERE link IN (SELECT id FROM link WHERE kind = 'db');
         DELETE FROM interval WHERE link IN (SELECT id FROM link WHERE kind = 'db');
         DELETE FROM link WHERE kind = 'db';",
    )?;
    // The table nodes, one per (service, name), keyed by the owner's key so they follow it.
    let mut nodes: BTreeMap<(i64, String), i64> = BTreeMap::new();
    for (_, def) in links.keys() {
        nodes.entry((def.1, def.0.clone())).or_insert(0);
    }
    for ((service, name), id) in &mut nodes {
        let owner: String =
            conn.query_row("SELECT key FROM node WHERE id = ?1", [service], |r| {
                r.get(0)
            })?;
        let rest = owner.split_once(':').map_or(owner.as_str(), |x| x.1);
        *id = conn.query_row(
            "INSERT INTO node (kind, key, name, parent, service) VALUES ('table', ?1, ?2, ?3, ?3)
             ON CONFLICT (key) DO UPDATE SET parent = excluded.parent, service = excluded.service
             RETURNING id",
            params![format!("table:{rest}:{name}"), name, service],
            |r| r.get(0),
        )?;
    }
    let mut written = 0;
    for ((src, def), acc) in links {
        let types: Vec<Ev> = acc.types.into_iter().collect();
        let c = confidence::score(&types);
        conn.prepare_cached(
            "INSERT INTO link (kind, src, dst, level, score) VALUES ('db', ?1, ?2, ?3, ?4)",
        )?
        .execute(params![
            src,
            nodes[&(def.1, def.0.clone())],
            c.level.as_str(),
            c.score
        ])?;
        let id = conn.last_insert_rowid();
        for (from, to) in merge(acc.spans) {
            conn.prepare_cached(
                "INSERT INTO interval (link, from_seq, to_seq) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![id, from, to])?;
            written += 1;
        }
        for (file, kind, text, span, operation) in &acc.sites {
            conn.prepare_cached(
                "INSERT INTO evidence (link, file, kind, text, start_line, start_col, end_line,
                   end_col, extractor, operation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                id,
                file,
                kind,
                cut(text),
                span[0],
                span[1],
                span[2],
                span[3],
                VERSION,
                operation
            ])?;
        }
    }
    record(conn, "tables", "workspace", &k)?;
    Ok(written)
}
