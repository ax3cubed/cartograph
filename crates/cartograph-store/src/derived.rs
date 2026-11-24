//! Derived rows and what they were built from (IDX-21, B6).
//!
//! Every derived artefact (a service's structure, the counts, a parent's layout, the search rows,
//! later summaries) has one row in `derived`: its kind, its scope and an inputs key. A run
//! recomputes the key from what the artefact reads and rebuilds the artefact only when the key
//! changed. AI output is never deleted on a change: it is marked stale and served as stale.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// What an artefact is built from. Inputs are sorted before hashing, so their order never matters.
#[derive(Debug, Clone, Default)]
pub struct KeyInputs<'a> {
    /// Input blob ids, upstream fact hashes or other canonical bytes.
    pub inputs: Vec<Vec<u8>>,
    /// The extractors (name, version) whose output it reads.
    pub extractors: Vec<(&'a str, &'a str)>,
    pub config_hash: Option<&'a str>,
    /// For AI rows: the model id and the prompt version.
    pub model: Option<(&'a str, &'a str)>,
}

fn field(h: &mut Sha256, b: &[u8]) {
    h.update((b.len() as u64).to_le_bytes());
    h.update(b);
}

/// `SHA-256(kind ‖ sorted inputs ‖ extractor names and versions ‖ config hash ‖ model id and
/// prompt version)` (B6), each part length-prefixed so no two lists encode the same.
pub fn inputs_key(kind: &str, k: &KeyInputs<'_>) -> [u8; 32] {
    let mut h = Sha256::new();
    field(&mut h, b"cartograph-inputs-key v1");
    field(&mut h, kind.as_bytes());
    let mut inputs: Vec<&[u8]> = k.inputs.iter().map(Vec::as_slice).collect();
    inputs.sort_unstable();
    h.update((inputs.len() as u64).to_le_bytes());
    for i in inputs {
        field(&mut h, i);
    }
    let mut ex = k.extractors.clone();
    ex.sort_unstable();
    h.update((ex.len() as u64).to_le_bytes());
    for (n, v) in ex {
        field(&mut h, n.as_bytes());
        field(&mut h, v.as_bytes());
    }
    field(&mut h, k.config_hash.unwrap_or("").as_bytes());
    let (m, p) = k.model.unwrap_or(("", ""));
    field(&mut h, m.as_bytes());
    field(&mut h, p.as_bytes());
    h.finalize().into()
}

/// The stored key of an artefact, if it was built.
pub fn stored(conn: &Connection, kind: &str, scope: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.prepare_cached("SELECT inputs_key FROM derived WHERE kind = ?1 AND scope = ?2")?
        .query_row(params![kind, scope], |r| r.get(0))
        .optional()
}

/// True when the artefact must be rebuilt: it was never built, or built from other inputs.
pub fn changed(
    conn: &Connection,
    kind: &str,
    scope: &str,
    key: &[u8; 32],
) -> rusqlite::Result<bool> {
    Ok(stored(conn, kind, scope)?.as_deref() != Some(&key[..]))
}

/// Records that the artefact was built from `key`.
pub fn record(conn: &Connection, kind: &str, scope: &str, key: &[u8; 32]) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO derived (kind, scope, inputs_key, stale) VALUES (?1, ?2, ?3, 0)
         ON CONFLICT (kind, scope) DO UPDATE SET inputs_key = excluded.inputs_key, stale = 0",
    )?
    .execute(params![kind, scope, &key[..]])
    .map(drop)
}

/// Forgets an artefact (its scope is gone).
pub fn forget(conn: &Connection, kind: &str, scope: &str) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM derived WHERE kind = ?1 AND scope = ?2")?
        .execute(params![kind, scope])
        .map(drop)
}

/// For AI output (summaries, concepts): a new key marks the row stale, keeping it and its old key,
/// instead of deleting it. Returns true when it became stale. A row built from `key` is left fresh.
pub fn mark_stale_if_changed(
    conn: &Connection,
    kind: &str,
    scope: &str,
    key: &[u8; 32],
) -> rusqlite::Result<bool> {
    Ok(conn
        .prepare_cached(
            "UPDATE derived SET stale = 1 WHERE kind = ?1 AND scope = ?2 AND inputs_key != ?3
               AND stale = 0",
        )?
        .execute(params![kind, scope, &key[..]])?
        > 0)
}

/// Whether an artefact is stale (AI output built from inputs that have since changed).
pub fn is_stale(conn: &Connection, kind: &str, scope: &str) -> rusqlite::Result<Option<bool>> {
    conn.prepare_cached("SELECT stale FROM derived WHERE kind = ?1 AND scope = ?2")?
        .query_row(params![kind, scope], |r| r.get(0))
        .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn key(inputs: &[&[u8]], config: Option<&str>, model: Option<(&str, &str)>) -> [u8; 32] {
        inputs_key(
            "layout",
            &KeyInputs {
                inputs: inputs.iter().map(|i| i.to_vec()).collect(),
                extractors: vec![("ts-imports/ts", "1.0.0")],
                config_hash: config,
                model,
            },
        )
    }

    #[test]
    fn every_part_of_a_key_counts() {
        let base = key(&[b"a", b"b"], Some("c1"), None);
        assert_ne!(base, key(&[b"a"], Some("c1"), None), "inputs");
        assert_ne!(base, key(&[b"a", b"b"], Some("c2"), None), "config hash");
        assert_ne!(
            base,
            key(&[b"a", b"b"], Some("c1"), Some(("m", "1"))),
            "model"
        );
        assert_ne!(
            key(&[b"ab"], None, None),
            key(&[b"a", b"b"], None, None),
            "fields are length-prefixed"
        );
        let other = inputs_key(
            "layout",
            &KeyInputs {
                inputs: vec![b"a".to_vec(), b"b".to_vec()],
                extractors: vec![("ts-imports/ts", "1.0.1")],
                config_hash: Some("c1"),
                model: None,
            },
        );
        assert_ne!(base, other, "extractor version");
        assert_ne!(base, inputs_key("counts", &KeyInputs::default()), "kind");
    }

    proptest! {
        #[test]
        fn reordering_inputs_never_changes_a_key(
            mut v in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..8), 0..12),
            seed in any::<u64>(),
        ) {
            let k = |v: &[Vec<u8>]| inputs_key("k", &KeyInputs { inputs: v.to_vec(), ..Default::default() });
            let before = k(&v);
            // A deterministic shuffle.
            let n = v.len();
            for i in (1..n).rev() {
                let j = (seed.wrapping_mul(i as u64 + 7) % (i as u64 + 1)) as usize;
                v.swap(i, j);
            }
            prop_assert_eq!(before, k(&v));
        }
    }

    #[test]
    fn a_new_model_marks_a_summary_stale_without_deleting_it() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE derived (kind TEXT NOT NULL, scope TEXT NOT NULL, inputs_key BLOB NOT NULL,
               stale INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (kind, scope)) STRICT, WITHOUT ROWID;",
        )
        .unwrap();
        let m1 = key(&[b"blob"], None, Some(("model-a", "p1")));
        let m2 = key(&[b"blob"], None, Some(("model-b", "p1")));
        record(&c, "summary", "n1", &m1).unwrap();
        assert!(!mark_stale_if_changed(&c, "summary", "n1", &m1).unwrap());
        assert_eq!(is_stale(&c, "summary", "n1").unwrap(), Some(false));
        assert!(mark_stale_if_changed(&c, "summary", "n1", &m2).unwrap());
        assert_eq!(
            is_stale(&c, "summary", "n1").unwrap(),
            Some(true),
            "kept, stale"
        );
        assert_eq!(
            stored(&c, "summary", "n1").unwrap().as_deref(),
            Some(&m1[..])
        );
        record(&c, "summary", "n1", &m2).unwrap();
        assert_eq!(
            is_stale(&c, "summary", "n1").unwrap(),
            Some(false),
            "rebuilt"
        );
    }
}
