//! Generated clients (XS-11, H5): a call to a function of a generated client is a call to the spec
//! operation whose method and path that function spells out.
//!
//! Two facts of the blob say what is needed, both from built-in query adapters (`cartograph_facts`):
//! `codegen.marker` (the file's banner: orval, openapi-typescript, openapi-generator, protoc-gen,
//! the Go `Code generated ... DO NOT EDIT.` line) and `symbol.def` (a function of a generated
//! client with the literal method and path it calls). The alias table is therefore a fact of the
//! generated blob, computed once per regeneration. A def is matched to the operation of a spec by
//! method and path, never by name, so a regenerated client that renames its functions keeps its
//! `client_alias` rows (the name only says which call sites reach it).
//!
//! What a file calls is read only from the files that import a generated client, when the import
//! links exist: `symbol.ref` facts of the `refs` adapters, one per name per blob, kept in `blob_ref`. Nothing is
//! evaluated; a config file of a generator is never read, and no spec is chosen by one: a def meets
//! every operation of every spec it matches, which is what its literal path says.

use cartograph_facts::adapters::{AdapterSet, Limits};
use cartograph_facts::schema::{AttrValue, BlobFact, FactKind, Part};
use cartograph_git::GitError;
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};

use crate::derive::Inputs;
use crate::ifaces;
use crate::links::http::{Seg, normalise_route};
use crate::rows::cut;
use crate::structure::merge;

/// `refs@1.0.0+refs-go@1.0.0`: changes whenever one of those adapters does.
pub fn refs_version() -> String {
    AdapterSet::refs()
        .adapters()
        .iter()
        .map(|a| format!("{}@{}", a.name, a.version))
        .collect::<Vec<_>>()
        .join("+")
}

/// An interval of commits: `to` is `None` while it lasts.
pub(crate) type Span = (i64, Option<i64>);

fn meet(a: Span, b: Span) -> Option<Span> {
    let from = a.0.max(b.0);
    let to = match (a.1, b.1) {
        (None, t) | (t, None) => t,
        (Some(x), Some(y)) => Some(x.min(y)),
    };
    to.is_none_or(|t| t > from).then_some((from, to))
}

/// A function of a generated client and the method and path it spells out.
pub(crate) struct Def {
    pub file: i64,
    pub service: i64,
    /// The names callers use for it.
    pub names: Vec<String>,
    pub method: String,
    pub segs: Vec<Seg>,
    pub line: u32,
    pub span: Span,
}

/// A name a file calls.
pub(crate) struct Ref {
    pub file: i64,
    pub service: i64,
    pub name: String,
    pub at: [u32; 4],
    pub extractor: String,
    pub span: Span,
}

/// An operation, as the contracts pass wrote it.
pub(crate) struct Op {
    pub id: i64,
    pub method: String,
    pub path: String,
    pub segs: Vec<Seg>,
    pub span: Span,
}

/// A generated function that is an operation, over the time both held.
pub(crate) struct Alias {
    pub op: i64,
    pub file: i64,
    pub symbol: String,
    pub line: u32,
    pub span: Span,
}

/// A call through an alias: the call site in the caller's file, with the operation's method and
/// path as if it spelled them out.
pub(crate) struct Call {
    pub file: i64,
    pub service: i64,
    pub at: [u32; 4],
    pub method: String,
    pub path: String,
    pub name: String,
    pub extractor: String,
    pub span: Span,
}

/// What a blob says about generated code.
#[derive(Default)]
pub(crate) struct Found {
    pub generator: Option<String>,
    pub defs: Vec<(Vec<String>, String, String, u32)>,
    pub refs: Vec<(String, [u32; 4], String)>,
}

/// The text of a value, a path's expressions as `${}`.
fn text(v: &AttrValue, exprs: bool) -> String {
    match v {
        AttrValue::Str(s) => s.as_str().to_owned(),
        AttrValue::Parts { parts } => parts
            .as_slice()
            .iter()
            .map(|p| match p {
                Part::Lit(s) => s.as_str(),
                Part::Expr(s) if exprs => s.as_str(),
                Part::Expr(_) => "${}",
            })
            .collect(),
    }
}

/// A generated name and the other names callers use for it: oapi-codegen builds the request in
/// `NewGetOrderRequest` and callers use `GetOrder` or `GetOrderWithResponse`.
fn names(symbol: &str) -> Vec<String> {
    match symbol
        .strip_prefix("New")
        .and_then(|r| r.strip_suffix("Request"))
    {
        Some(op) if !op.is_empty() => vec![
            symbol.to_owned(),
            op.to_owned(),
            format!("{op}WithResponse"),
        ],
        _ => vec![symbol.to_owned()],
    }
}

/// A def of generated gRPC code (XS-18): `RPC` for one rpc, `SERVE` for a service.
fn grpc(attrs: &cartograph_facts::schema::Attrs) -> bool {
    matches!(attrs.get("method"), Some(AttrValue::Str(s)) if matches!(s.as_str(), "RPC" | "SERVE"))
}

/// The names callers use for a generated gRPC def: `client.Get` (Go) and `client.get` (connect-es)
/// for the rpc `/pkg.Orders/Get`, and `RegisterOrdersServer` for the service `/pkg.Orders`.
fn grpc_names(method: &str, path: &str) -> Vec<String> {
    let last = |s: &'static str, p: &str| p.rsplit(s).next().unwrap_or_default().to_owned();
    if method == "SERVE" {
        return vec![format!("Register{}Server", last(".", path))];
    }
    let m = last("/", path);
    let mut lower = m.clone();
    if let Some(c) = lower.get_mut(..1) {
        c.make_ascii_lowercase();
    }
    let mut v = vec![m];
    if v[0] != lower {
        v.push(lower);
    }
    v
}

/// What the adapters' facts of one blob say, in `found`.
pub(crate) fn read(payload: &[u8], found: &mut Found) {
    let has = |n: &[u8]| payload.windows(n.len()).any(|w| w == n);
    if !(has(b"\"codegen.marker\"") || has(b"\"symbol.def\"") || has(b"\"symbol.ref\"")) {
        return;
    }
    let Some(per) = ifaces::decode(payload) else {
        return;
    };
    for (adapter, version, facts) in per {
        for f in facts {
            let attr = |k: &str, exprs| f.attrs.get(k).map(|v| text(v, exprs));
            match f.kind {
                FactKind::CodegenMarker => {
                    found
                        .generator
                        .get_or_insert(attr("generator", true).unwrap_or_default());
                }
                FactKind::SymbolDef if grpc(&f.attrs) => {
                    let (Some(method), Some(path)) = (
                        attr("method", true),
                        attr("path", false).or_else(|| {
                            Some(format!(
                                "{}/{}",
                                attr("service", false)?,
                                attr("rpc", false)?
                            ))
                        }),
                    ) else {
                        continue;
                    };
                    let path = format!("/{}", path.trim_start_matches('/'));
                    found
                        .defs
                        .push((grpc_names(&method, &path), method, path, f.span.0[0]));
                }
                FactKind::SymbolDef => {
                    let (Some(symbol), Some(method), Some(path)) = (
                        attr("symbol", true),
                        attr("method", true),
                        attr("path", false),
                    ) else {
                        continue;
                    };
                    found.defs.push((names(&symbol), method, path, f.span.0[0]));
                }
                FactKind::SymbolRef => {
                    if let Some(name) = attr("name", true) {
                        found
                            .refs
                            .push((name, f.span.0, format!("{adapter}@{version}")));
                    }
                }
                _ => {}
            }
        }
    }
}

/// Aliases and the calls through them. A call is a name a file of the same service calls that is a
/// name of a def, while the def's method and path are an operation of a spec (the path may carry a
/// base the generated code leaves to its client).
pub(crate) fn link(defs: &[Def], refs: &[Ref], ops: &[Op]) -> (Vec<Alias>, Vec<Call>) {
    let mut aliases = Vec::new();
    let mut calls = Vec::new();
    for d in defs {
        for o in ops {
            if o.method != d.method || d.segs.is_empty() || !o.segs.ends_with(&d.segs) {
                continue;
            }
            let Some(span) = meet(d.span, o.span) else {
                continue;
            };
            for name in &d.names {
                aliases.push(Alias {
                    op: o.id,
                    file: d.file,
                    symbol: name.clone(),
                    line: d.line,
                    span,
                });
                for r in refs
                    .iter()
                    .filter(|r| r.name == *name && r.service == d.service && r.file != d.file)
                {
                    let Some(span) = meet(span, r.span) else {
                        continue;
                    };
                    calls.push(Call {
                        file: r.file,
                        service: r.service,
                        at: r.at,
                        method: o.method.clone(),
                        path: o.path.clone(),
                        name: r.name.clone(),
                        extractor: r.extractor.clone(),
                        span,
                    });
                }
            }
        }
    }
    (aliases, calls)
}

/// A route of a gRPC service that a file registers (XS-18): `RegisterOrdersServer(s, impl)` serves
/// every rpc of the service the generated code says it registers.
pub(crate) struct Served {
    pub file: i64,
    pub service: i64,
    pub at: [u32; 4],
    pub op: i64,
    pub method: String,
    pub path: String,
    pub name: String,
    pub extractor: String,
    pub span: Span,
}

/// The routes of the services that files of the same service register, by the operations of the
/// protos that have the service's rpcs.
pub(crate) fn serve(defs: &[Def], refs: &[Ref], ops: &[Op]) -> Vec<Served> {
    let mut out = Vec::new();
    for d in defs
        .iter()
        .filter(|d| d.method == "serve" && !d.segs.is_empty())
    {
        for o in ops
            .iter()
            .filter(|o| o.method == "rpc" && o.segs.starts_with(&d.segs))
        {
            let Some(span) = meet(d.span, o.span) else {
                continue;
            };
            for r in refs
                .iter()
                .filter(|r| d.names.contains(&r.name) && r.service == d.service && r.file != d.file)
            {
                let Some(span) = meet(span, r.span) else {
                    continue;
                };
                out.push(Served {
                    file: r.file,
                    service: r.service,
                    at: r.at,
                    op: o.id,
                    method: o.method.clone(),
                    path: o.path.clone(),
                    name: r.name.clone(),
                    extractor: r.extractor.clone(),
                    span,
                });
            }
        }
    }
    out
}

/// A def's method and path as the matcher reads them.
pub(crate) fn segs(method: &str, path: &str) -> (String, Vec<Seg>) {
    let t = normalise_route(method, path);
    (t.method, t.segs)
}

/// Reads what the files that import a generated client call, once per blob, into `fact` rows of
/// the `refs` extractor. The links exist already: the replay made them, and the generated file is
/// known by its marker.
pub(crate) fn extract_refs<E: From<GitError> + From<rusqlite::Error>>(
    conn: &Connection,
    inputs: &Inputs<'_>,
) -> Result<(), E> {
    // No generated client in the index: nothing imports one. Scanning every adapter payload for a
    // marker costs a warm sync ~15 ms, so the count kept by `ifaces::write` answers instead; an
    // index without it (made before it existed) is counted once here.
    let known: Option<i64> = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = ?1",
            [ifaces::CODEGEN_BLOBS],
            |r| r.get(0),
        )
        .optional()?;
    let marked = match known {
        Some(n) => n,
        None => {
            let n: i64 = conn.query_row(
                "SELECT count(*) FROM fact fa JOIN extractor e ON e.id = fa.extractor AND e.name = ?1
                 WHERE instr(fa.payload, CAST('codegen.marker' AS BLOB)) > 0",
                [ifaces::EXTRACTOR],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                params![ifaces::CODEGEN_BLOBS, n],
            )?;
            n
        }
    };
    if marked == 0 {
        return Ok(());
    }
    let version = refs_version();
    // Skipped when nothing that could add an importer of a generated client was added since: a
    // new import needs a new file version, and a replay after a rewrite gives new row ids.
    let mut counts: Vec<i64> = ["fact", "link", "file_version", "blob_ref"]
        .iter()
        .map(|t| conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)))
        .collect::<Result<_, _>>()?;
    for t in ["link", "file_version"] {
        counts.push(conn.query_row(
            &format!("SELECT coalesce(max(rowid), 0) FROM {t}"),
            [],
            |r| r.get(0),
        )?);
    }
    let k = crate::derive::key(
        "blob_ref",
        "blob_ref 1",
        vec![
            format!("{counts:?}").into_bytes(),
            version.clone().into_bytes(),
        ],
        inputs.config_hash,
    );
    if !changed(conn, "blob_ref", "workspace", &k)? {
        return Ok(());
    }
    let todo: Vec<(Vec<u8>, String, i64)> = conn
        .prepare_cached(
            "WITH gen AS (
               SELECT DISTINCT fv.file FROM fact fa
                 JOIN extractor e ON e.id = fa.extractor AND e.name = ?1
                 JOIN file_version fv ON fv.blob = fa.blob
               WHERE instr(fa.payload, CAST('codegen.marker' AS BLOB)) > 0)
             , target AS (
               SELECT g.id FROM gen JOIN node g ON g.file = gen.file AND g.kind = 'file'
               UNION
               -- A Go import names a package: the module node of the folder.
               SELECT m.id FROM gen JOIN file_path fp ON fp.file = gen.file
                 JOIN node m ON m.key = 'module:' || rtrim(rtrim(fp.path, replace(fp.path, '/', '')), '/'))
             SELECT DISTINCT fv.blob, (SELECT path FROM file_path WHERE file = s.file
                                       ORDER BY from_seq DESC LIMIT 1), f.repo
             FROM target t
               JOIN link l ON l.dst = t.id AND l.kind IN ('import', 'package')
               JOIN node s ON s.id = l.src AND s.file IS NOT NULL
               JOIN file f ON f.id = s.file
               JOIN file_version fv ON fv.file = s.file
             WHERE NOT EXISTS (SELECT 1 FROM blob_ref x WHERE x.blob = fv.blob AND x.version = ?2)
             ORDER BY fv.blob",
        )?
        .query_map(params![ifaces::EXTRACTOR, version], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;
    let limits = Limits::default();
    for (blob, path, repo) in todo {
        let Some(r) = inputs.repos.iter().find(|r| r.id == repo) else {
            continue;
        };
        let Ok(oid) = cartograph_git::ObjectId::try_from(blob.as_slice()) else {
            continue;
        };
        let bytes = cartograph_git::read_blob(&r.repo.local(), oid, limits.max_bytes as u64).ok();
        let got: Vec<(String, String, Vec<BlobFact>)> = bytes
            .map(|b| AdapterSet::refs().run(&path, &b, &limits))
            .unwrap_or_default()
            .into_iter()
            .filter(|a| !a.facts.is_empty())
            .map(|a| (a.name, a.version, a.facts))
            .collect();
        conn.prepare_cached(
            "INSERT OR REPLACE INTO blob_ref (blob, version, payload) VALUES (?1, ?2, ?3)",
        )?
        .execute(params![
            blob,
            version,
            serde_json::to_vec(&got).unwrap_or_default()
        ])?;
    }
    // The counts after this run's writes are what the next run compares with.
    let after: Vec<i64> = ["fact", "link", "file_version", "blob_ref"]
        .iter()
        .map(|t| conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)))
        .collect::<Result<_, _>>()?;
    let k = crate::derive::key(
        "blob_ref",
        "blob_ref 1",
        vec![format!("{after:?}").into_bytes(), version.into_bytes()],
        inputs.config_hash,
    );
    record(conn, "blob_ref", "workspace", &k)?;
    Ok(())
}

/// The alias rows of a run, a function's stretches of time merged.
pub(crate) fn write_aliases(conn: &Connection, aliases: &[Alias]) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM client_alias", [])?;
    let mut by: std::collections::BTreeMap<(i64, i64, &str, u32), Vec<Span>> = Default::default();
    for a in aliases {
        by.entry((a.op, a.file, a.symbol.as_str(), a.line))
            .or_default()
            .push(a.span);
    }
    for ((op, file, symbol, line), spans) in by {
        for (from, to) in merge(spans) {
            conn.prepare_cached(
                "INSERT INTO client_alias (operation, file, symbol, line, from_seq, to_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![op, file, cut(symbol), line, from, to])?;
        }
    }
    Ok(())
}
