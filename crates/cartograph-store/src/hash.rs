//! The fact hash (IDX-17, B4): SHA-256 over a canonical list of an index's facts, so two indexes
//! built from the same commits compare equal whatever their page size, insert order, SQLite version
//! or machine. It proves equal facts, not where they came from: it is not a signature.
//!
//! Canonical input (§4.9), as sections hashed in this order:
//!
//! - `extractors`: extractor names and versions, sorted.
//! - `config`: `meta.config_hash`, or nothing.
//! - `facts`: (blob id, extractor name, encoded payload), sorted by blob and extractor. Facts belong
//!   to blobs, which repos share, so they are one section rather than one per repo.
//! - per repo, sorted by root commit (then name, for two repos with one root):
//!   - `repo N commit`: the indexed commit;
//!   - `repo N lineages`: each file's paths and versions with the shas that opened and closed them;
//!   - `repo N intervals`: (kind, source key, target key, opening sha, closing sha or empty).
//!
//! A node's key is its repo's position N in that order and the first path of its file's lineage.
//! No row ids, seq numbers, timestamps or local paths reach the hash. Each record is a list of
//! length-prefixed fields; each section's records are sorted, then hashed; the total is the hash of
//! the header and every (label, section digest) pair. Keeping the section digests lets `verify`
//! name the first section that differs.
//!
//! Interval origin (IDX-21) joins the interval record when the index stores one, with the header
//! moving to v2.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::{Index, StoreError};

pub const HEADER: &[u8] = b"cartograph-fact-hash v1\n";

/// One section of the canonical input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// What is hashed: `repo 0 intervals`.
    pub label: String,
    /// What a person reads: `repo 0 (acme-api) intervals`. Not hashed.
    pub shown: String,
    pub digest: [u8; 32],
}

/// An index's fact hash, section by section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactHash {
    pub sections: Vec<Section>,
}

impl FactHash {
    /// The hash itself.
    pub fn digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(HEADER);
        for s in &self.sections {
            field(&mut h, s.label.as_bytes());
            h.update(s.digest);
        }
        h.finalize().into()
    }

    /// The first section that differs from `other`, as a person reads it.
    pub fn first_difference(&self, other: &FactHash) -> Option<String> {
        let mut a = self.sections.iter();
        let mut b = other.sections.iter();
        loop {
            match (a.next(), b.next()) {
                (None, None) => return None,
                (Some(x), Some(y)) if x.label == y.label && x.digest == y.digest => {}
                (Some(x), Some(y)) if x.label == y.label => return Some(x.shown.clone()),
                (Some(x), _) => return Some(x.shown.clone()),
                (None, Some(y)) => return Some(y.shown.clone()),
            }
        }
    }
}

/// `sha256:<hex>`, as `meta.fact_hash` stores it.
impl fmt::Display for FactHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        self.digest().iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

fn field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// One record: its fields, each length-prefixed, so no two field lists encode the same.
fn record(fields: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(fields.iter().map(|f| f.len() + 8).sum());
    for f in fields {
        v.extend((f.len() as u64).to_le_bytes());
        v.extend(*f);
    }
    v
}

/// The fact hash of an open index.
pub fn fact_hash(conn: &Connection) -> rusqlite::Result<FactHash> {
    let section = |label: &str, digest| Section {
        label: label.into(),
        shown: label.into(),
        digest,
    };
    let mut sections = Vec::new();
    let mut st = conn.prepare("SELECT name, version FROM extractor ORDER BY name, version")?;
    let rows: Vec<Vec<u8>> = st
        .query_map([], |r| {
            Ok(record(&[
                r.get_ref(0)?.as_bytes()?,
                r.get_ref(1)?.as_bytes()?,
            ]))
        })?
        .collect::<Result<_, _>>()?;
    sections.push(section("extractors", digest(rows)));

    let config: Option<Vec<u8>> = conn
        .query_row(
            "SELECT CAST(value AS BLOB) FROM meta WHERE key = 'config_hash'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    sections.push(section(
        "config",
        digest(config.into_iter().map(|c| record(&[&c])).collect()),
    ));

    let mut st = conn.prepare(
        "SELECT f.blob, e.name, e.version, f.payload FROM fact f JOIN extractor e ON e.id = f.extractor
         ORDER BY f.blob, e.name, e.version",
    )?;
    std::thread::scope(|scope| {
        // Fact records stream to another thread in chunks (a bounded queue: the facts are the
        // bulk of an index) and are hashed there in order, while the rest is read.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<Vec<u8>>>(8);
        let facts = scope.spawn(move || {
            let mut h = Sha256::new();
            for r in rx.iter().flatten() {
                field(&mut h, &r);
            }
            h.finalize().into()
        });
        let mut chunk = Vec::with_capacity(FACT_CHUNK);
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            chunk.push(record(&[
                r.get_ref(0)?.as_bytes()?,
                r.get_ref(1)?.as_bytes()?,
                r.get_ref(2)?.as_bytes()?,
                r.get_ref(3)?.as_bytes()?,
            ]));
            if chunk.len() == FACT_CHUNK {
                let full = std::mem::replace(&mut chunk, Vec::with_capacity(FACT_CHUNK));
                tx.send(full)
                    .expect("the hasher runs until the queue closes");
            }
        }
        drop(rows);
        tx.send(chunk)
            .expect("the hasher runs until the queue closes");
        drop(tx);
        let rest = repo_sections(conn, scope)?;
        sections.push(section("facts", facts.join().expect("no panic")));
        for (label, shown, job) in rest {
            sections.push(Section {
                label,
                shown,
                digest: job.join().expect("no panic"),
            });
        }
        Ok(FactHash { sections })
    })
}

/// Fact records per message to the hashing thread.
const FACT_CHUNK: usize = 256;

/// The digest of records in this order.
fn digest(records: Vec<Vec<u8>>) -> [u8; 32] {
    let mut h = Sha256::new();
    for r in records {
        field(&mut h, &r);
    }
    h.finalize().into()
}

type Job<'scope> = std::thread::ScopedJoinHandle<'scope, [u8; 32]>;

/// Each repository's sections, in order, as (label, shown, digest being computed).
fn repo_sections<'scope>(
    conn: &Connection,
    scope: &'scope std::thread::Scope<'scope, '_>,
) -> rusqlite::Result<Vec<(String, String, Job<'scope>)>> {
    // Repos in root-commit order; N is the position.
    let mut st = conn.prepare(
        "SELECT r.id, r.name, r.indexed_commit,
                (SELECT sha FROM commits c WHERE c.repo = r.id ORDER BY seq LIMIT 1) AS root
         FROM repo r ORDER BY root, r.name",
    )?;
    let repos: Vec<(i64, String, Option<Vec<u8>>)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let position: HashMap<i64, usize> = repos
        .iter()
        .enumerate()
        .map(|(n, (id, ..))| (*id, n))
        .collect();

    // Each step's sha. A row whose opening step has none is left out, as an inner join would;
    // a closing step without one reads as open.
    let shas: HashMap<i64, Vec<u8>> = conn
        .prepare("SELECT seq, sha FROM commits")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;

    // Each file's key: its repo's position and its lineage's first path (by step, then path).
    let mut first: HashMap<i64, (i64, String)> = HashMap::new();
    let mut st = conn.prepare("SELECT file, from_seq, path FROM file_path")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (file, from): (i64, i64) = (r.get(0)?, r.get(1)?);
        let path = r.get_ref(2)?.as_str()?;
        match first.get_mut(&file) {
            Some(f) if (from, path) < (f.0, f.1.as_str()) => *f = (from, path.to_owned()),
            Some(_) => {}
            None => {
                first.insert(file, (from, path.to_owned()));
            }
        }
    }
    drop(rows);
    let mut st = conn.prepare("SELECT id, repo FROM file")?;
    let mut files: HashMap<i64, (usize, Vec<u8>)> = HashMap::new();
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let repo = position
            .get(&r.get::<_, i64>(1)?)
            .copied()
            .unwrap_or(usize::MAX);
        let path = first.get(&id).map_or("", |f| f.1.as_str());
        files.insert(id, (repo, format!("{repo}:{path}").into_bytes()));
    }
    drop(rows);

    let mut lineages: Vec<Vec<Vec<u8>>> = vec![Vec::new(); repos.len()];
    for (tag, sql) in [
        (
            b"path".as_slice(),
            "SELECT file, CAST(path AS BLOB), from_seq, to_seq FROM file_path",
        ),
        (
            b"version".as_slice(),
            "SELECT file, blob, from_seq, to_seq FROM file_version",
        ),
    ] {
        let mut st = conn.prepare(sql)?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some(from) = shas.get(&r.get::<_, i64>(2)?) else {
                continue;
            };
            let Some((repo, key)) = files.get(&r.get::<_, i64>(0)?) else {
                continue;
            };
            let to = r.get::<_, Option<i64>>(3)?.and_then(|s| shas.get(&s));
            let rec = record(&[
                key,
                tag,
                r.get_ref(1)?.as_bytes()?,
                from,
                to.map_or(&[][..], Vec::as_slice),
            ]);
            if let Some(v) = lineages.get_mut(*repo) {
                v.push(rec);
            }
        }
    }

    // A node's key is its file's; a node without a file (a Go package's folder) is keyed by its
    // structure key, which names its path.
    let mut nodes: HashMap<i64, (usize, Vec<u8>)> = HashMap::new();
    let mut st = conn.prepare("SELECT id, coalesce(key, kind), file FROM node")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let node = match r.get::<_, Option<i64>>(2)?.and_then(|f| files.get(&f)) {
            Some((repo, key)) => (*repo, key.clone()),
            None => (usize::MAX, r.get::<_, String>(1)?.into_bytes()),
        };
        nodes.insert(r.get(0)?, node);
    }
    drop(rows);
    let mut intervals: Vec<Vec<Vec<u8>>> = vec![Vec::new(); repos.len()];
    let mut st = conn.prepare(
        "SELECT l.kind, l.src, l.dst, x.from_seq, x.to_seq FROM interval x
         JOIN link l ON l.id = x.link WHERE l.kind NOT IN ('http', 'event')",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (Some((repo, src)), Some((_, dst)), Some(from)) = (
            nodes.get(&r.get::<_, i64>(1)?),
            nodes.get(&r.get::<_, i64>(2)?),
            shas.get(&r.get::<_, i64>(3)?),
        ) else {
            continue;
        };
        let to = r.get::<_, Option<i64>>(4)?.and_then(|s| shas.get(&s));
        let rec = record(&[
            r.get_ref(0)?.as_bytes()?,
            src,
            dst,
            from,
            to.map_or(&[][..], Vec::as_slice),
        ]);
        if let Some(v) = intervals.get_mut(*repo) {
            v.push(rec);
        }
    }
    drop(rows);
    let mut out = Vec::new();
    for (n, ((_, name, indexed), (lin, int))) in repos
        .iter()
        .zip(lineages.into_iter().zip(intervals))
        .enumerate()
    {
        let shown = |what: &str| format!("repo {n} ({name}) {what}");
        let commit = vec![record(&[indexed.as_deref().unwrap_or_default()])];
        out.push((
            format!("repo {n} commit"),
            shown("indexed commit"),
            scope.spawn(move || digest(commit)),
        ));
        // ponytail: lineages and intervals are sorted in memory (about 200k small records at the
        // 412 MB scale), each section on its own thread. The plan's covering-index stream comes
        // with the 2 s budget check in HD-09.
        for (what, shown_as, mut records) in
            [("lineages", "files", lin), ("intervals", "links", int)]
        {
            out.push((
                format!("repo {n} {what}"),
                shown(shown_as),
                scope.spawn(move || {
                    records.sort_unstable();
                    digest(records)
                }),
            ));
        }
    }
    Ok(out)
}

/// Computes the fact hash and stores it in `meta.fact_hash`, in the caller's transaction.
pub fn store_fact_hash(conn: &Connection) -> rusqlite::Result<FactHash> {
    let h = fact_hash(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('fact_hash', ?1)",
        [h.to_string()],
    )?;
    Ok(h)
}

/// What `cartograph verify` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ours: String,
    pub theirs: String,
    /// None when the hashes are equal; otherwise the first section that differs, as shown.
    pub first_difference: Option<String>,
}

/// Compares the facts of two index files. Both go through the open rules first, since the other
/// index may come from anywhere.
pub fn verify(ours: &Path, theirs: &Path) -> Result<Verdict, StoreError> {
    let hash = |p: &Path| -> Result<FactHash, StoreError> {
        let ix = Index::open_read(p)?;
        fact_hash(&ix.conn).map_err(|source| StoreError::Sqlite {
            path: p.display().to_string(),
            source,
        })
    };
    let (a, b) = (hash(ours)?, hash(theirs)?);
    Ok(Verdict {
        ours: a.to_string(),
        theirs: b.to_string(),
        first_difference: a.first_difference(&b),
    })
}
