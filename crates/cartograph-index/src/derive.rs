//! The derived rows of a run (IDX-21, B6): structure (IDX-20), counts and fan-in, search (AI-15)
//! and layout (IDX-25), each rebuilt only when its inputs key changed.
//!
//! Keys come from what is already at hand at the end of a run: a repository's tip and manifest
//! entries (detection), its path rows' fingerprint and roots (structure), the fact hash (counts and
//! fan-in), each parent's children and weights (layout). Search rows are refreshed for the lineages
//! the run moved, and in full when the search key changes.

use std::collections::{HashMap, HashSet};

use cartograph_facts::Replay;
use cartograph_git::{GitError, ObjectId, Repo};
use cartograph_parse::{BlobFacts, SkipReason, extractor_for};
use cartograph_store::FactHash;
use cartograph_store::derived::{KeyInputs, changed, inputs_key, record};
use cartograph_store::rusqlite::{self, Connection, params};

use crate::aliases;
use crate::layout::{self, Circle};
use crate::owners;
use crate::structure::{self, Detected, Root, ServiceSpec};

/// Bump when what an artefact holds changes, so every index rebuilds it.
const DETECT_VERSION: &str = "detect 2";
const STRUCTURE_VERSION: &str = "structure 2";
const COUNTS_VERSION: &str = "counts 1";
const SEARCH_VERSION: &str = "search 1";
const OWNERS_VERSION: &str = "owners 1";

/// One repository as the derived rows see it.
pub struct RepoInput<'a> {
    pub id: i64,
    pub name: &'a str,
    /// Its paths carry `<name>/` (a multi-repository index).
    pub prefixed: bool,
    pub repo: &'a Repo,
    /// The indexed commit.
    pub tip: ObjectId,
    pub scopes: &'a [String],
}

/// What a run's derived rows are built from besides the index.
#[derive(Default)]
pub struct Inputs<'a> {
    pub repos: Vec<RepoInput<'a>>,
    pub services: &'a [ServiceSpec],
    /// The manifest's `contracts:` entries (XS-08).
    pub contracts: &'a [crate::contracts::ContractSpec],
    /// The manifest's `environments:` and `default_environment` (XS-09).
    pub environments: &'a [crate::config::EnvSpec],
    pub default_environment: Option<&'a str>,
    pub exclude: &'a [String],
    pub config_hash: Option<&'a str>,
    /// Lineages that got a new path this run.
    pub moved: HashSet<i64>,
    /// Positions to carry into a rebuilt index, by node key.
    pub carried: Option<HashMap<String, Circle>>,
    /// Repositories dropped from the workspace: (id, name, prefixed).
    pub removed: Vec<(i64, String, bool)>,
}

/// What [`refresh`] rebuilt: artefacts whose key changed, and rows written for them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DerivedStats {
    pub structures: usize,
    pub counts: usize,
    pub search_rows: usize,
    pub layouts: usize,
    pub owners: usize,
}

impl DerivedStats {
    pub fn rebuilt(&self) -> usize {
        self.structures + self.counts + self.search_rows + self.layouts + self.owners
    }
}

/// The coverage totals of a run (IDX-22): live tracked source files, those parsed, live import
/// references and those that resolved.
pub fn coverage(replay: &Replay) -> (i64, i64, i64, i64) {
    let (mut total, mut parsed) = (0, 0);
    for (path, f) in replay.tree() {
        if extractor_for(path).is_some_and(|(_, k)| !k.starts_with("manifest-")) {
            // A generated file isn't a gap: nobody means to read it (XS-11).
            if matches!(**f, BlobFacts::Skipped(SkipReason::Generated)) {
                continue;
            }
            total += 1;
            parsed += i64::from(!matches!(**f, BlobFacts::Skipped(_)));
        }
    }
    let (l, e, b, u) = replay.outcomes();
    let refs = (l + e + b + u) as i64;
    (total, parsed, refs, refs - u as i64)
}

/// What `cartograph status` prints about coverage (IDX-22, board A01).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub files_total: i64,
    pub files_parsed: i64,
    pub imports_total: i64,
    pub imports_resolved: i64,
    /// Open unresolved imports: (path, line, specifier). The specifier is untrusted text (F8).
    pub unresolved: Vec<(String, i64, String)>,
    /// Unresolved rows past the cap, counted only.
    pub dropped: i64,
}

/// The last run's coverage totals and every import unresolved at the last step, by path and line.
pub fn coverage_report(conn: &Connection) -> rusqlite::Result<Option<CoverageReport>> {
    use cartograph_store::rusqlite::OptionalExtension;
    type Totals = [Option<i64>; 4];
    let totals: Option<Totals> = conn
        .query_row(
            "SELECT files_total, files_parsed, imports_total, imports_resolved FROM run
             ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
        )
        .optional()?;
    let Some(
        [
            Some(files_total),
            Some(files_parsed),
            Some(imports_total),
            Some(imports_resolved),
        ],
    ) = totals
    else {
        return Ok(None);
    };
    let unresolved = conn
        .prepare(
            "SELECT fp.path, u.line, u.spec FROM unresolved u
             JOIN file_path fp ON fp.file = u.file AND fp.to_seq IS NULL
             WHERE u.to_seq IS NULL AND u.fact_kind IS NULL ORDER BY fp.path, u.line, u.spec",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let dropped = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'unresolved_dropped'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(Some(CoverageReport {
        files_total,
        files_parsed,
        imports_total,
        imports_resolved,
        unresolved,
        dropped,
    }))
}

pub(crate) fn key(
    kind: &str,
    version: &str,
    inputs: Vec<Vec<u8>>,
    config: Option<&str>,
) -> [u8; 32] {
    inputs_key(
        kind,
        &KeyInputs {
            inputs,
            extractors: vec![(kind, version)],
            config_hash: config,
            model: None,
        },
    )
}

/// The roots a structure pass last wrote for a repository, read back from its service nodes.
fn stored_roots(conn: &Connection, repo: &str) -> rusqlite::Result<Vec<Root>> {
    let prefix = format!("service:{repo}:");
    let mut st = conn.prepare(
        "SELECT key, name, kind, detected_by, detect_source FROM node
         WHERE key >= ?1 AND key < ?2 AND detected_by IS NOT NULL ORDER BY key",
    )?;
    let rows = st.query_map(params![prefix, format!("service:{repo};")], |r| {
        let key: String = r.get(0)?;
        let kind: String = r.get(2)?;
        Ok(Root {
            path: key[prefix.len()..].to_owned(),
            name: r.get(1)?,
            kind: if kind == "library" {
                "library"
            } else {
                "service"
            },
            detected_by: r.get(3)?,
            source: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// What a repository's structure is built from: its roots, its path rows' fingerprint and whether
/// its paths are prefixed. The whole-repository root and the remainder are the same root, and
/// `build` adds it when no root covers the top: it is left out either way.
fn structure_inputs(roots: &[Root], fingerprint: Vec<u8>, prefixed: bool) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = roots
        .iter()
        .filter(|x| !(x.path.is_empty() && x.detected_by == "repo"))
        .map(|x| {
            let source = x.source.as_deref().unwrap_or("");
            format!(
                "{}\0{}\0{}\0{}\0{source}",
                x.path, x.name, x.kind, x.detected_by
            )
            .into_bytes()
        })
        .collect();
    v.push(fingerprint);
    v.push(format!("prefixed {prefixed}").into_bytes());
    v
}

/// A cheap fingerprint of a repository's path rows: any path added, moved, closed or removed
/// changes it.
fn path_fingerprint(conn: &Connection, repo: i64) -> rusqlite::Result<Vec<u8>> {
    conn.query_row(
        "SELECT count(*), coalesce(max(fp.rowid), 0), total(fp.from_seq),
                total(coalesce(fp.to_seq, -1)), total(length(fp.path))
         FROM file_path fp JOIN file f ON f.id = fp.file WHERE f.repo = ?1",
        [repo],
        |r| {
            Ok(format!(
                "{}:{}:{}:{}:{}",
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?
            )
            .into_bytes())
        },
    )
}

/// Rebuilds the derived rows whose inputs changed, at the run's last step `seq`, in the caller's
/// transaction.
pub fn refresh<E: From<GitError> + From<rusqlite::Error>>(
    conn: &Connection,
    inputs: &Inputs<'_>,
    hash: &FactHash,
    seq: i64,
) -> Result<DerivedStats, E> {
    let mut st = DerivedStats::default();
    let config = inputs.config_hash;
    let mut structure_keys: Vec<Vec<u8>> = Vec::new();
    let mut restructured = false;
    for r in &inputs.repos {
        let mine: Vec<&ServiceSpec> = inputs
            .services
            .iter()
            .filter(|s| s.repo == r.name)
            .collect();
        let mut detect_in = vec![r.tip.as_bytes().to_vec()];
        detect_in.extend(r.scopes.iter().map(|s| format!("scope\0{s}").into_bytes()));
        detect_in.extend(mine.iter().map(|s| {
            format!("service\0{}\0{}", s.name, s.path.as_deref().unwrap_or("")).into_bytes()
        }));
        detect_in.extend(
            inputs
                .exclude
                .iter()
                .map(|e| format!("exclude\0{e}").into_bytes()),
        );
        let dk = key("detect", DETECT_VERSION, detect_in, config);
        let detected: Option<Detected> = if changed(conn, "detect", r.name, &dk)? {
            let d = structure::detect(
                r.repo,
                r.tip,
                r.name,
                r.scopes,
                inputs.services,
                inputs.exclude,
            )?;
            record(conn, "detect", r.name, &dk)?;
            Some(d)
        } else {
            None
        };
        let roots = match &detected {
            Some(d) => d.roots.clone(),
            None => stored_roots(conn, r.name)?,
        };
        let s_in = structure_inputs(&roots, path_fingerprint(conn, r.id)?, r.prefixed);
        let sk = key("structure", STRUCTURE_VERSION, s_in, config);
        if detected.is_some() || changed(conn, "structure", r.name, &sk)? {
            structure::build(conn, r.id, r.name, r.prefixed.then_some(r.name), &roots)?;
            record(conn, "structure", r.name, &sk)?;
            st.structures += 1;
            restructured = true;
        }
        aliases::refresh(conn, r.id, r.name, r.prefixed.then_some(r.name), config)?;
        structure_keys.push(sk.to_vec());

        // Owners (XS-23): only when a CODEOWNERS version or a path row changed.
        owners::scan::<E>(conn, r.id, r.repo)?;
        let mut o_in = vec![
            owners::versions_key(conn, r.id)?,
            format!("prefixed {}", r.prefixed).into_bytes(),
        ];
        if o_in[0].is_empty() {
            o_in.truncate(1);
        } else {
            o_in.push(path_fingerprint(conn, r.id)?);
        }
        let ok = key("owners", OWNERS_VERSION, o_in, None);
        if changed(conn, "owners", r.name, &ok)? {
            owners::build::<E>(conn, r.id, r.repo, r.prefixed.then_some(r.name))?;
            record(conn, "owners", r.name, &ok)?;
            st.owners += 1;
        }
    }

    // A repository dropped from the workspace keeps its structure; its spans close with its files.
    for (id, name, prefixed) in &inputs.removed {
        let roots = stored_roots(conn, name)?;
        let s_in = structure_inputs(&roots, path_fingerprint(conn, *id)?, *prefixed);
        let sk = key("structure", STRUCTURE_VERSION, s_in, config);
        if changed(conn, "structure", name, &sk)? {
            structure::build(conn, *id, name, prefixed.then_some(name.as_str()), &roots)?;
            record(conn, "structure", name, &sk)?;
            st.structures += 1;
            restructured = true;
        }
        structure_keys.push(sk.to_vec());
    }

    // Contracts (XS-08) need the services; the HTTP pass binds them to routes and calls.
    crate::contracts::refresh::<E>(conn, inputs)?;
    // What the files that import a generated client call (XS-11).
    crate::contracts::codegen::extract_refs::<E>(conn, inputs)?;
    // Config values and environments (XS-09) come from every repository's value-source files.
    crate::config::refresh(conn, inputs.environments, inputs.default_environment)?;
    // HTTP links (XS-04) need every repository's services and host aliases, and the config values.
    crate::http::refresh(conn, config)?;

    let mut c_in = structure_keys;
    c_in.push(hash.digest().to_vec());
    let ck = key("counts", COUNTS_VERSION, c_in, config);
    let recount = changed(conn, "counts", "workspace", &ck)?;
    if recount {
        counts(conn)?;
        record(conn, "counts", "workspace", &ck)?;
        st.counts += 1;
    }

    let sk = key("search", SEARCH_VERSION, Vec::new(), None);
    let full = changed(conn, "search", "workspace", &sk)?;
    st.search_rows += search_rows(conn, full, restructured, &inputs.moved)?;
    if full {
        record(conn, "search", "workspace", &sk)?;
    }

    if restructured || recount || full {
        st.layouts += layout::run(conn, seq, inputs.carried.as_ref())?;
    }
    Ok(st)
}

/// Fan-in and per-scope counts at the last step (IDX-21): what find ranks by and what the service
/// and module levels show.
fn counts(conn: &Connection) -> rusqlite::Result<()> {
    let links: Vec<(i64, i64)> = conn
        .prepare("SELECT l.src, l.dst FROM interval i JOIN link l ON l.id = i.link WHERE i.to_seq IS NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut fan_in: HashMap<i64, i64> = HashMap::new();
    for (_, d) in &links {
        *fan_in.entry(*d).or_default() += 1;
    }
    let had: Vec<(i64, i64)> = conn
        .prepare("SELECT id, fan_in FROM node WHERE fan_in != 0")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (id, n) in had {
        if fan_in.get(&id) != Some(&n) {
            conn.prepare_cached("UPDATE node SET fan_in = ?2 WHERE id = ?1")?
                .execute(params![id, fan_in.get(&id).copied().unwrap_or(0)])?;
        }
        fan_in.remove(&id);
    }
    for (id, n) in fan_in {
        conn.prepare_cached("UPDATE node SET fan_in = ?2 WHERE id = ?1")?
            .execute(params![id, n])?;
    }

    // Each live node's module (its parent, or itself) and service.
    let mut up: HashMap<i64, (Option<i64>, Option<i64>)> = HashMap::new();
    let mut st = conn.prepare(
        "SELECT n.id, n.kind, n.parent, n.service FROM node n
         WHERE (n.kind = 'file' AND EXISTS (SELECT 1 FROM file_path fp WHERE fp.file = n.file
                                            AND fp.to_seq IS NULL))
            OR (n.kind != 'file' AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id
                                             AND s.to_seq IS NULL))",
    )?;
    let mut scopes: HashMap<i64, [i64; 5]> = HashMap::new();
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (id, kind, parent, service): (i64, String, Option<i64>, Option<i64>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        match kind.as_str() {
            "file" => {
                let module = parent.filter(|p| Some(*p) != service);
                up.insert(id, (module, service));
                for s in [module, service].into_iter().flatten() {
                    scopes.entry(s).or_default()[1] += 1;
                }
            }
            "module" => {
                up.insert(id, (Some(id), service));
                scopes.entry(id).or_default();
                if let Some(s) = service {
                    scopes.entry(s).or_default()[0] += 1;
                }
            }
            _ => {
                up.insert(id, (None, Some(id)));
                scopes.entry(id).or_default();
            }
        }
    }
    drop(rows);
    for (s, d) in &links {
        let (Some(a), Some(b)) = (up.get(s), up.get(d)) else {
            continue;
        };
        for (x, y) in [(a.0, b.0), (a.1, b.1)] {
            match (x, y) {
                (Some(x), Some(y)) if x == y => scopes.entry(x).or_default()[4] += 1,
                (x, y) => {
                    if let Some(x) = x {
                        scopes.entry(x).or_default()[2] += 1;
                    }
                    if let Some(y) = y {
                        scopes.entry(y).or_default()[3] += 1;
                    }
                }
            }
        }
    }
    conn.execute("DELETE FROM scope_count", [])?;
    let mut ids: Vec<_> = scopes.into_iter().collect();
    ids.sort_unstable();
    for (id, [m, f, o, i, x]) in ids {
        conn.prepare_cached(
            "INSERT INTO scope_count (node, modules, files, links_out, links_in, links_inside)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![id, m, f, o, i, x])?;
    }
    Ok(())
}

/// Search rows: every node when `full`, else the moved lineages and, after a structure pass, the
/// structure nodes. A file is found by its last path.
fn search_rows(
    conn: &Connection,
    full: bool,
    structure: bool,
    moved: &HashSet<i64>,
) -> rusqlite::Result<usize> {
    let mut n = 0;
    let mut st = conn.prepare(
        "SELECT n.id, n.file, (SELECT path FROM file_path p WHERE p.file = n.file
                               ORDER BY from_seq DESC LIMIT 1)
         FROM node n WHERE n.kind = 'file'",
    )?;
    if full || !moved.is_empty() {
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let (id, file): (i64, i64) = (r.get(0)?, r.get(1)?);
            if !full && !moved.contains(&file) {
                continue;
            }
            let Some(path) = r.get::<_, Option<String>>(2)? else {
                continue;
            };
            let name = path.rsplit('/').next().unwrap_or(&path);
            cartograph_store::search::put(conn, id, name, &path)?;
            n += 1;
        }
    }
    if full || structure {
        let rows: Vec<(i64, String, String)> = conn
            .prepare("SELECT id, coalesce(name, ''), key FROM node WHERE kind != 'file' AND key IS NOT NULL")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        for (id, name, key) in rows {
            let path = key.split_once(':').map_or(key.as_str(), |(_, p)| p);
            cartograph_store::search::put(conn, id, &name, path)?;
            n += 1;
        }
    }
    Ok(n)
}
