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

/// Accumulates one section.
struct Builder {
    sections: Vec<Section>,
}

impl Builder {
    /// A section of already-ordered records.
    fn ordered(&mut self, label: String, shown: String, records: impl Iterator<Item = Vec<u8>>) {
        let mut h = Sha256::new();
        for r in records {
            field(&mut h, &r);
        }
        self.sections.push(Section {
            label,
            shown,
            digest: h.finalize().into(),
        });
    }

    /// A section of records in any order.
    ///
    /// ponytail: lineages and intervals are sorted in memory (about 200k small records at the 412 MB
    /// scale). The plan's covering-index stream comes with the 2 s budget check in HD-09.
    fn sorted(&mut self, label: String, shown: String, mut records: Vec<Vec<u8>>) {
        records.sort_unstable();
        self.ordered(label, shown, records.into_iter());
    }
}

/// The fact hash of an open index.
pub fn fact_hash(conn: &Connection) -> rusqlite::Result<FactHash> {
    let mut b = Builder {
        sections: Vec::new(),
    };
    let mut st = conn.prepare("SELECT name, version FROM extractor ORDER BY name, version")?;
    let rows: Vec<Vec<u8>> = st
        .query_map([], |r| {
            Ok(record(&[
                r.get_ref(0)?.as_bytes()?,
                r.get_ref(1)?.as_bytes()?,
            ]))
        })?
        .collect::<Result<_, _>>()?;
    b.ordered("extractors".into(), "extractors".into(), rows.into_iter());

    let config: Option<Vec<u8>> = conn
        .query_row(
            "SELECT CAST(value AS BLOB) FROM meta WHERE key = 'config_hash'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    b.ordered(
        "config".into(),
        "config".into(),
        config.into_iter().map(|c| record(&[&c])),
    );

    let mut st = conn.prepare(
        "SELECT f.blob, e.name, e.version, f.payload FROM fact f JOIN extractor e ON e.id = f.extractor
         ORDER BY f.blob, e.name, e.version",
    )?;
    let mut h = Sha256::new();
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let rec = record(&[
            r.get_ref(0)?.as_bytes()?,
            r.get_ref(1)?.as_bytes()?,
            r.get_ref(2)?.as_bytes()?,
            r.get_ref(3)?.as_bytes()?,
        ]);
        field(&mut h, &rec);
    }
    b.sections.push(Section {
        label: "facts".into(),
        shown: "facts".into(),
        digest: h.finalize().into(),
    });

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

    // Each file's key: its repo's position and its lineage's first path.
    let mut st = conn.prepare(
        "SELECT f.id, f.repo,
                (SELECT path FROM file_path p WHERE p.file = f.id ORDER BY from_seq, path LIMIT 1)
         FROM file f",
    )?;
    let mut files: HashMap<i64, (usize, Vec<u8>)> = HashMap::new();
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let repo = position
            .get(&r.get::<_, i64>(1)?)
            .copied()
            .unwrap_or(usize::MAX);
        let path: Option<String> = r.get(2)?;
        let key = format!("{repo}:{}", path.unwrap_or_default()).into_bytes();
        files.insert(r.get(0)?, (repo, key));
    }

    let mut lineages: Vec<Vec<Vec<u8>>> = vec![Vec::new(); repos.len()];
    const SHAS: &str =
        "JOIN commits cf ON cf.seq = x.from_seq LEFT JOIN commits ct ON ct.seq = x.to_seq";
    for (tag, sql) in [
        (
            b"path".as_slice(),
            format!("SELECT x.file, CAST(x.path AS BLOB), cf.sha, ct.sha FROM file_path x {SHAS}"),
        ),
        (
            b"version".as_slice(),
            format!("SELECT x.file, x.blob, cf.sha, ct.sha FROM file_version x {SHAS}"),
        ),
    ] {
        let mut st = conn.prepare(&sql)?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some((repo, key)) = files.get(&r.get::<_, i64>(0)?) else {
                continue;
            };
            let to: Option<Vec<u8>> = r.get(3)?;
            let rec = record(&[
                key,
                tag,
                r.get_ref(1)?.as_bytes()?,
                r.get_ref(2)?.as_bytes()?,
                to.as_deref().unwrap_or_default(),
            ]);
            if let Some(v) = lineages.get_mut(*repo) {
                v.push(rec);
            }
        }
    }

    // A node's key is its file's; a node without a file (none yet) is keyed by its kind.
    let mut intervals: Vec<Vec<Vec<u8>>> = vec![Vec::new(); repos.len()];
    let mut st = conn.prepare(
        "SELECT l.kind, ns.kind, ns.file, nd.kind, nd.file, cf.sha, ct.sha
         FROM interval x JOIN link l ON l.id = x.link
         JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
         JOIN commits cf ON cf.seq = x.from_seq LEFT JOIN commits ct ON ct.seq = x.to_seq",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let node = |kind: usize, file: usize| -> rusqlite::Result<(usize, Vec<u8>)> {
            Ok(
                match r.get::<_, Option<i64>>(file)?.and_then(|f| files.get(&f)) {
                    Some((repo, key)) => (*repo, key.clone()),
                    None => (usize::MAX, r.get::<_, String>(kind)?.into_bytes()),
                },
            )
        };
        let (repo, src) = node(1, 2)?;
        let (_, dst) = node(3, 4)?;
        let to: Option<Vec<u8>> = r.get(6)?;
        let rec = record(&[
            r.get_ref(0)?.as_bytes()?,
            &src,
            &dst,
            r.get_ref(5)?.as_bytes()?,
            to.as_deref().unwrap_or_default(),
        ]);
        if let Some(v) = intervals.get_mut(repo) {
            v.push(rec);
        }
    }

    for (n, ((_, name, indexed), (lin, int))) in repos
        .iter()
        .zip(lineages.into_iter().zip(intervals))
        .enumerate()
    {
        let shown = |what: &str| format!("repo {n} ({name}) {what}");
        b.ordered(
            format!("repo {n} commit"),
            shown("indexed commit"),
            std::iter::once(record(&[indexed.as_deref().unwrap_or_default()])),
        );
        b.sorted(format!("repo {n} lineages"), shown("files"), lin);
        b.sorted(format!("repo {n} intervals"), shown("links"), int);
    }
    Ok(FactHash {
        sections: b.sections,
    })
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
