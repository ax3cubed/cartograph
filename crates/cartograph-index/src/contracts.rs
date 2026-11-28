//! Contracts (XS-08): OpenAPI and Swagger specs as rows, with intervals.
//!
//! The `manifest-openapi` extractor reads each spec blob once and says its operations, schemas and
//! unresolved refs as facts (see `cartograph_parse::openapi`). Here those facts become rows: each
//! version of each spec file gives its operations an interval, the time that version was at its
//! path, like every other row of the index. The owning service is the one named by a manifest
//! `contracts:` entry that matches the file, else the service whose root holds it.
//!
//! A spec that reads other files through relative `$ref`s is read again here, with a [`Files`]
//! over the git tree of each commit it was live at (never the file system): the spec has a new
//! state at each commit where one of the files it read changed. Refs that leave the repository
//! or name a remote URL are not followed; they become `unresolved` rows.
//!
//! Rows are rebuilt only when the spec facts, the service they map to, or (for a spec with
//! relative refs) the repository's tip changed. The binding of operations to routes and calls
//! (`binding` rows, drift) is the HTTP pass's work (`crate::http`), which reads these rows.

use std::collections::BTreeMap;

use cartograph_git::{GitError, ObjectId, Repo, read_blob};
use cartograph_parse::openapi::{self, Contract, Env, Files, MAX_SPEC_BYTES};
use cartograph_parse::openapi_diff::{canonical, diff};
use cartograph_parse::{BlobFacts, Extractor, ManifestOpenapi, ManifestProto, proto};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, params};
use sha2::{Digest, Sha256};

pub(crate) mod codegen;

use crate::derive::{Inputs, key};
use crate::links::http::normalise_route;
use crate::rows::cut;
use crate::structure::{dir, merge};
use crate::timeline::glob_matches;

/// Most breaking changes kept per version; the count says how many there were.
const MAX_BREAKING: usize = 200;

/// Bump when what the rows hold changes, so every index rebuilds them.
const CONTRACTS_VERSION: &str = "contracts 3";

/// The extractor name in the `fact` table.
pub const EXTRACTOR: &str = openapi::KIND;

/// One `contracts:` entry of the manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractSpec {
    /// A glob relative to each repository's root.
    pub path: String,
    /// The service that owns the specs it matches.
    pub service: Option<String>,
}

impl From<&cartograph_discovery::manifest::Contract> for ContractSpec {
    fn from(c: &cartograph_discovery::manifest::Contract) -> Self {
        Self {
            path: c.path.clone(),
            service: c.service.clone(),
        }
    }
}

/// `openapi@1.0.0`: what evidence rows name as their extractor.
pub fn extractor() -> String {
    format!("openapi@{}", ManifestOpenapi.version())
}

/// The key the contract rows were last built under, for passes that read them.
pub(crate) fn built_key(conn: &Connection) -> rusqlite::Result<Vec<u8>> {
    Ok(conn
        .query_row(
            "SELECT inputs_key FROM derived WHERE kind = 'contracts' AND scope = 'workspace'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default())
}

/// One version of a spec file at a path: the overlap of the two intervals, and its facts.
struct Version {
    repo: i64,
    file: i64,
    /// The file node's service.
    service: Option<i64>,
    /// As in `file_path` (with the repository's prefix in a multi-repository index).
    path: String,
    /// Repository-relative.
    rel: String,
    blob: Vec<u8>,
    from: i64,
    to: Option<i64>,
    contract: Contract,
}

fn versions(
    conn: &Connection,
    prefixes: &BTreeMap<i64, Option<&str>>,
) -> rusqlite::Result<Vec<Version>> {
    let mut st = conn.prepare_cached(
        "SELECT f.repo, fp.file, n.service, fp.path, fp.from_seq, fp.to_seq, fv.blob,
                fv.from_seq, fv.to_seq, fa.payload
         FROM fact fa JOIN extractor e ON e.id = fa.extractor AND e.name IN (?1, ?2)
           JOIN file_version fv ON fv.blob = fa.blob
           JOIN file_path fp ON fp.file = fv.file
           JOIN file f ON f.id = fp.file
           JOIN node n ON n.file = fp.file AND n.kind = 'file'
         ORDER BY fp.path, fp.from_seq, fv.from_seq, fv.blob",
    )?;
    let mut rows = st.query([EXTRACTOR, proto::KIND])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let from = r.get::<_, i64>(4)?.max(r.get(7)?);
        let to = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<i64>>(8)?) {
            (None, t) | (t, None) => t,
            (Some(a), Some(b)) => Some(a.min(b)),
        };
        if to.is_some_and(|t| t <= from) {
            continue;
        }
        let Some(BlobFacts::Facts(facts)) = BlobFacts::decode(r.get_ref(9)?.as_blob()?) else {
            continue;
        };
        let Some(contract) = openapi::from_facts(&facts) else {
            continue;
        };
        let repo: i64 = r.get(0)?;
        let path: String = r.get(3)?;
        let rel = match prefixes.get(&repo).copied().flatten() {
            Some(x) => path
                .strip_prefix(x)
                .map_or(path.as_str(), |r| r.trim_start_matches('/')),
            None => &path,
        }
        .to_owned();
        out.push(Version {
            repo,
            file: r.get(1)?,
            service: r.get(2)?,
            path,
            rel,
            blob: r.get(6)?,
            from,
            to,
            contract,
        });
    }
    Ok(out)
}

/// The files of a repository at one commit, through its git tree.
struct GitFiles<'a> {
    repo: &'a Repo,
    commit: ObjectId,
}

impl Files for GitFiles<'_> {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let oid = self
            .repo
            .first_blobs_at(&[self.commit], &[path])
            .ok()?
            .pop()??;
        read_blob(&self.repo.local(), oid, MAX_SPEC_BYTES).ok()
    }
}

/// A spec over a stretch of history.
struct State {
    from: i64,
    to: Option<i64>,
    contract: Contract,
}

/// The states of a spec file version: one, unless it reads other files, in which case one more at
/// each commit where a file it read changed. `commits` are the repository's, in order.
fn states(repo: &Repo, v: &Version, commits: &[(i64, ObjectId)]) -> Result<Vec<State>, GitError> {
    // A proto file's imports are resolved once, against the tip, in `refresh`.
    if v.contract.files.is_empty() || proto::named(&v.rel) {
        return Ok(vec![State {
            from: v.from,
            to: v.to,
            contract: v.contract.clone(),
        }]);
    }
    let Ok(blob) = ObjectId::try_from(v.blob.as_slice()) else {
        return Ok(Vec::new());
    };
    let bytes = read_blob(&repo.local(), blob, MAX_SPEC_BYTES)?;
    let commits: Vec<(i64, ObjectId)> = commits
        .iter()
        .filter(|c| c.0 >= v.from && v.to.is_none_or(|t| c.0 < t))
        .copied()
        .collect();
    let ids: Vec<ObjectId> = commits.iter().map(|c| c.1).collect();
    let folder = dir(&v.rel).to_owned();
    let mut out: Vec<State> = Vec::new();
    let mut at = 0;
    while at < commits.len() {
        let parsed = openapi::parse(
            &bytes,
            Some(Env {
                dir: &folder,
                files: &mut GitFiles {
                    repo,
                    commit: commits[at].1,
                },
            }),
        );
        let Ok(Some(contract)) = parsed else { break };
        // The next commit where any file read differs from what this one saw.
        let mut next = commits.len();
        for f in contract.files.iter().take(64) {
            let blobs = repo.first_blobs_at(&ids[at..], &[f.as_str()])?;
            if let Some(i) = blobs.iter().position(|b| *b != blobs[0]) {
                next = next.min(at + i);
            }
        }
        out.push(State {
            from: commits[at].0,
            to: commits.get(next).map(|c| c.0).or(v.to),
            contract,
        });
        at = next;
    }
    Ok(out)
}

/// The imports of a live `.proto` file that don't resolve inside the repository (XS-18): each is
/// tried against the roots a `buf.yaml` or `buf.work.yaml` above the file names, then every folder
/// above it, and only to files the index has at its tip. `buf` keeps each folder's buf roots.
fn import_refs(
    conn: &Connection,
    r: &crate::derive::RepoInput<'_>,
    v: &Version,
    prefixes: &BTreeMap<i64, Option<&str>>,
    buf: &mut BTreeMap<(i64, String), Vec<String>>,
    imports: &[String],
) -> Vec<(String, &'static str)> {
    let mut files = GitFiles {
        repo: r.repo,
        commit: r.tip,
    };
    let mut roots: Vec<String> = Vec::new();
    let above = proto::ancestors(dir(&v.rel));
    for a in &above {
        let found = buf.entry((v.repo, a.clone())).or_insert_with(|| {
            ["buf.yaml", "buf.work.yaml"]
                .iter()
                .filter_map(|n| {
                    files.read(&if a.is_empty() {
                        (*n).into()
                    } else {
                        format!("{a}/{n}")
                    })
                })
                .flat_map(|y| proto::buf_roots(a, &y))
                .collect()
        });
        roots.extend(found.iter().cloned());
    }
    roots.extend(above);
    let mut seen = std::collections::BTreeSet::new();
    roots.retain(|x| seen.insert(x.clone()));
    let prefix = prefixes.get(&v.repo).copied().flatten();
    let mut exists = |p: &str| {
        let p = prefix.map_or_else(|| p.to_owned(), |x| format!("{x}/{p}"));
        conn.prepare_cached("SELECT 1 FROM file_path WHERE path = ?1 AND to_seq IS NULL")
            .and_then(|mut s| s.exists([p]))
            .unwrap_or(false)
    };
    imports
        .iter()
        .filter_map(|i| match proto::resolve(i, &roots, &mut exists) {
            Err("bundled") | Ok(_) => None,
            Err(why) => Some((i.clone(), why)),
        })
        .collect()
}

type Span = (i64, Option<i64>);

/// An operation over the versions of its spec; the last version's text wins.
#[derive(Default)]
struct OpAcc {
    template: String,
    operation_id: Option<String>,
    summary: Option<String>,
    line: u32,
    spans: Vec<Span>,
}

/// What the pass writes for one spec file.
#[derive(Default)]
struct Parts {
    title: String,
    version: String,
    /// An AsyncAPI document (XS-16): the contract's kind is `events`.
    events: bool,
    service: Option<i64>,
    /// (method, path with its base) -> the operation.
    ops: BTreeMap<(String, String), OpAcc>,
    schemas: BTreeMap<String, (String, Vec<Span>)>,
    unresolved: BTreeMap<(String, String), Vec<Span>>,
}

/// Rebuilds the `contract`, `operation` and `contract_schema` rows when the spec facts, the
/// services they map to or the contracts of the manifest changed. Returns the operation rows
/// written.
pub(crate) fn refresh<E: From<GitError> + From<rusqlite::Error>>(
    conn: &Connection,
    inputs: &Inputs<'_>,
) -> Result<usize, E> {
    let prefixes: BTreeMap<i64, Option<&str>> = inputs
        .repos
        .iter()
        .map(|r| (r.id, r.prefixed.then_some(r.name)))
        .collect();
    let all = versions(conn, &prefixes)?;

    // The service a manifest entry names, in the repository of the file when it has one there.
    let mut named: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    if !inputs.contracts.is_empty() {
        let mut st = conn.prepare_cached(
            "SELECT name, key, id FROM node WHERE detected_by IS NOT NULL AND name IS NOT NULL
             ORDER BY key",
        )?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
        })?;
        for r in rows {
            let (name, key, id): (String, String, i64) = r?;
            named.entry(name).or_default().push((key, id));
        }
    }
    let owner = |v: &Version| -> Option<i64> {
        let repo = inputs.repos.iter().find(|r| r.id == v.repo)?;
        let want = inputs
            .contracts
            .iter()
            .find(|c| glob_matches(&c.path, &v.rel))
            .and_then(|c| c.service.as_ref());
        match want {
            Some(name) => {
                let c = named.get(name)?;
                let mine = format!("service:{}:", repo.name);
                c.iter()
                    .find(|(k, _)| k.starts_with(&mine))
                    .or(c.first())
                    .map(|x| x.1)
            }
            None => v.service,
        }
    };

    let mut ins: Vec<Vec<u8>> = all
        .iter()
        .map(|v| {
            let mut b = format!(
                "{}\0{}\0{}\0{:?}\0{:?}\0{}\0",
                v.path,
                v.from,
                v.contract.operations.len(),
                v.to,
                owner(v),
                v.contract.files.len()
            )
            .into_bytes();
            b.extend_from_slice(&v.blob);
            b
        })
        .collect();
    // A spec that reads other files can change with a commit that doesn't touch the spec.
    for r in &inputs.repos {
        if all
            .iter()
            .any(|v| v.repo == r.id && !v.contract.files.is_empty() && !proto::named(&v.rel))
        {
            let mut b = format!("tip\0{}\0", r.name).into_bytes();
            b.extend_from_slice(r.tip.as_bytes());
            ins.push(b);
        }
    }
    ins.extend(
        inputs
            .contracts
            .iter()
            .map(|c| format!("glob\0{}\0{:?}", c.path, c.service).into_bytes()),
    );
    ins.push(ManifestOpenapi.version().as_bytes().to_vec());
    ins.push(ManifestProto.version().as_bytes().to_vec());
    let k = key("contracts", CONTRACTS_VERSION, ins, inputs.config_hash);
    if !changed(conn, "contracts", "workspace", &k)? {
        return Ok(0);
    }

    let mut parts: BTreeMap<i64, Parts> = BTreeMap::new();
    let mut commits: BTreeMap<i64, Vec<(i64, ObjectId)>> = BTreeMap::new();
    // Every state of each spec file (a file keeps its id across renames): where it began, the
    // root blob, the spec.
    let mut history: BTreeMap<i64, Vec<(i64, Vec<u8>, Contract)>> = BTreeMap::new();
    // The import roots a folder's buf files name, read once per folder.
    let mut buf: BTreeMap<(i64, String), Vec<String>> = BTreeMap::new();
    for v in &all {
        let Some(r) = inputs.repos.iter().find(|r| r.id == v.repo) else {
            continue;
        };
        if !v.contract.files.is_empty() && !proto::named(&v.rel) && !commits.contains_key(&v.repo) {
            let rows = conn
                .prepare_cached("SELECT seq, sha FROM commits WHERE repo = ?1 ORDER BY seq")?
                .query_map([v.repo], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            commits.insert(
                v.repo,
                rows.into_iter()
                    .filter_map(|(s, sha)| Some((s, ObjectId::try_from(sha.as_slice()).ok()?)))
                    .collect(),
            );
        }
        let none = Vec::new();
        for s in states(r.repo, v, commits.get(&v.repo).unwrap_or(&none))? {
            history
                .entry(v.file)
                .or_default()
                .push((s.from, v.blob.clone(), s.contract.clone()));
            let p = parts.entry(v.file).or_default();
            p.title = cut(&s.contract.title);
            p.version.clone_from(&s.contract.version);
            p.events = s.contract.events;
            p.service = owner(v);
            for o in &s.contract.operations {
                for base in &s.contract.bases {
                    let full = format!("{base}{}", o.path);
                    let e = p
                        .ops
                        .entry((o.method.clone(), full.clone()))
                        .or_insert_with(|| OpAcc {
                            // A channel address isn't a path: it is kept as the spec spells it.
                            template: if s.contract.events {
                                full.clone()
                            } else {
                                normalise_route(&o.method, &full).path()
                            },
                            ..Default::default()
                        });
                    e.operation_id.clone_from(&o.operation_id);
                    e.summary.clone_from(&o.summary);
                    e.line = o.line;
                    e.spans.push((s.from, s.to));
                }
            }
            for sc in &s.contract.schemas {
                let fields = serde_json::to_string(&sc.fields).unwrap_or_default();
                let e = p
                    .schemas
                    .entry(sc.name.clone())
                    .or_insert_with(|| (fields.clone(), Vec::new()));
                e.0 = fields;
                e.1.push((s.from, s.to));
            }
            for u in &s.contract.unresolved {
                p.unresolved
                    .entry((u.target.clone(), u.reason.clone()))
                    .or_default()
                    .push((s.from, s.to));
            }
            if proto::named(&v.rel) && s.to.is_none() {
                for (target, why) in import_refs(conn, r, v, &prefixes, &mut buf, &s.contract.files)
                {
                    p.unresolved
                        .entry((target, why.to_owned()))
                        .or_default()
                        .push((s.from, s.to));
                }
            }
        }
    }

    conn.execute_batch(
        "DELETE FROM contract_version; DELETE FROM binding; DELETE FROM client_alias; DELETE FROM contract_schema; DELETE FROM operation;
         DELETE FROM contract; DELETE FROM unresolved WHERE fact_kind = 'openapi.ref';",
    )?;
    let mut written = 0;
    for (file, p) in parts {
        conn.prepare_cached(
            "INSERT INTO contract (file, service, title, version, kind)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut s| {
            s.execute(params![
                file,
                p.service,
                p.title,
                p.version,
                if p.events { "events" } else { "openapi" }
            ])
        })?;
        let id = conn.last_insert_rowid();
        versions_of(conn, id, history.remove(&file).unwrap_or_default())?;
        for ((method, path), op) in &p.ops {
            for (from, to) in merge(op.spans.clone()) {
                conn.prepare_cached(
                    "INSERT INTO operation (contract, method, path, template, operation_id, summary,
                       line, from_seq, to_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .and_then(|mut s| {
                    s.execute(params![
                        id,
                        method,
                        cut(path),
                        format!("{method} {}", op.template),
                        op.operation_id,
                        op.summary,
                        op.line,
                        from,
                        to
                    ])
                })?;
                written += 1;
            }
        }
        for (name, (fields, spans)) in &p.schemas {
            for (from, to) in merge(spans.clone()) {
                conn.prepare_cached(
                    "INSERT INTO contract_schema (contract, name, fields, from_seq, to_seq)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .and_then(|mut s| s.execute(params![id, name, fields, from, to]))?;
            }
        }
        for ((target, reason), spans) in &p.unresolved {
            for (from, to) in merge(spans.clone()) {
                conn.prepare_cached(
                    "INSERT INTO unresolved (file, spec, line, stage, reason, from_seq, to_seq,
                       fact_kind, raw) VALUES (?1, ?2, 0, 'resolve', ?3, ?4, ?5, 'openapi.ref', ?2)",
                )
                .and_then(|mut s| s.execute(params![file, cut(target), reason, from, to]))?;
            }
        }
    }
    record(conn, "contracts", "workspace", &k)?;
    Ok(written)
}

/// The version rows of one contract: a new one wherever the canonical spec hash changes.
fn versions_of(
    conn: &Connection,
    contract: i64,
    mut states: Vec<(i64, Vec<u8>, Contract)>,
) -> rusqlite::Result<()> {
    states.sort_by_key(|s| s.0);
    let mut prev: Option<(Vec<u8>, Contract)> = None;
    let mut n = 0;
    for (seq, blob, c) in states {
        let hash = Sha256::digest(canonical(&c)).to_vec();
        if prev.as_ref().is_some_and(|p| p.0 == hash) {
            continue;
        }
        n += 1;
        let changes = prev.as_ref().map_or_else(Vec::new, |p| diff(&p.1, &c));
        // Names come from the spec: untrusted, masked like every other text column.
        let breaking: Vec<_> = changes
            .iter()
            .filter(|c| c.breaking)
            .take(MAX_BREAKING)
            .map(|c| cartograph_parse::openapi_diff::Change {
                subject: cut(&c.subject),
                summary: cut(&c.summary),
                ..c.clone()
            })
            .collect();
        let breaking_count = changes.iter().filter(|c| c.breaking).count();
        conn.prepare_cached(
            "INSERT INTO contract_version (contract, n, seq, blob, spec_hash, changes,
               breaking_count, breaking, summary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .and_then(|mut s| {
            s.execute(params![
                contract,
                n,
                seq,
                blob,
                hash,
                changes.len() as i64,
                breaking_count as i64,
                serde_json::to_string(&breaking).unwrap_or_default(),
                summary(&changes, prev.is_none().then_some(&c))
            ])
        })?;
        prev = Some((hash, c));
    }
    Ok(())
}

/// `Added POST /v1/orders; Removed GET /v1/x; +3 more`, from the diff alone.
pub fn summary(
    changes: &[cartograph_parse::openapi_diff::Change],
    first: Option<&Contract>,
) -> String {
    if let Some(c) = first {
        return format!("Initial spec, {} operations", c.operations.len());
    }
    if changes.is_empty() {
        return "Operation ids changed".into();
    }
    let mut s: Vec<String> = changes.iter().take(5).map(|c| c.summary.clone()).collect();
    if changes.len() > 5 {
        s.push(format!("+{} more", changes.len() - 5));
    }
    cut(&s.join("; "))
}
