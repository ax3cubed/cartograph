//! The fact cache, `<data>/cache/facts.db` (IDX-05, R2): facts by (blob id, extractor kind,
//! version), shared by every workspace. It is a cache: safe to delete, and an entry that doesn't
//! decode counts as a miss.

use std::path::Path;

use cartograph_parse::BlobFacts;
use rusqlite::{Connection, OptionalExtension};

use crate::schema::{self, BlobFact};

/// "CRTF".
const APPLICATION_ID: i32 = 0x4352_5446;

pub struct FactCache {
    conn: Connection,
}

impl FactCache {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            create_private_dir(dir)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA trusted_schema = OFF;
             PRAGMA application_id = {APPLICATION_ID};
             CREATE TABLE IF NOT EXISTS fact (
               blob BLOB NOT NULL, extractor TEXT NOT NULL, version TEXT NOT NULL, payload BLOB NOT NULL,
               PRIMARY KEY (blob, extractor, version)
             ) STRICT, WITHOUT ROWID;"
        ))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        Ok(Self { conn })
    }

    fn get_raw(&self, blob: &[u8], extractor: &str, version: &str) -> Option<Vec<u8>> {
        self.conn
            .prepare_cached(
                "SELECT payload FROM fact WHERE blob = ?1 AND extractor = ?2 AND version = ?3",
            )
            .ok()?
            .query_row((blob, extractor, version), |r| r.get::<_, Vec<u8>>(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn get(&self, blob: &[u8], extractor: &str, version: &str) -> Option<BlobFacts> {
        BlobFacts::decode(&self.get_raw(blob, extractor, version)?)
    }

    /// The interface facts (XS-01) of one blob for one adapter: one row holding one JSON array, so a
    /// blob is one read. Adapter names never collide with an extractor kind, so the rows share the
    /// table.
    pub fn get_interface(
        &self,
        blob: &[u8],
        extractor: &str,
        version: &str,
    ) -> Option<Vec<BlobFact>> {
        schema::decode(&self.get_raw(blob, extractor, version)?)
    }

    /// The interface facts of one blob for each of the adapters that apply to it, in one row.
    pub fn get_adapters(
        &self,
        blob: &[u8],
        key: &str,
        version: &str,
    ) -> Option<Vec<Vec<BlobFact>>> {
        serde_json::from_slice(&self.get_raw(blob, key, version)?).ok()
    }

    pub fn put_adapters(
        &mut self,
        blob: &[u8],
        key: &str,
        version: &str,
        facts: &[Vec<BlobFact>],
    ) -> rusqlite::Result<()> {
        let payload = serde_json::to_vec(facts).expect("facts always encode");
        self.put_raw_many([(blob, key, version, payload)])
    }

    /// Writes a batch in one transaction.
    pub fn put_many<'a>(
        &mut self,
        rows: impl IntoIterator<Item = (&'a [u8], &'a str, &'a str, &'a BlobFacts)>,
    ) -> rusqlite::Result<()> {
        self.put_raw_many(rows.into_iter().map(|(b, e, v, f)| (b, e, v, f.encode())))
    }

    /// Writes interface facts in one transaction.
    pub fn put_interface_many<'a>(
        &mut self,
        rows: impl IntoIterator<Item = (&'a [u8], &'a str, &'a str, &'a [BlobFact])>,
    ) -> rusqlite::Result<()> {
        self.put_raw_many(
            rows.into_iter()
                .map(|(b, e, v, f)| (b, e, v, schema::encode(f))),
        )
    }

    fn put_raw_many<'a>(
        &mut self,
        rows: impl IntoIterator<Item = (&'a [u8], &'a str, &'a str, Vec<u8>)>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "INSERT OR REPLACE INTO fact (blob, extractor, version, payload) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (blob, ex, ver, payload) in rows {
                st.execute((blob, ex, ver, payload))?;
            }
        }
        tx.commit()
    }
}

/// Creates a folder (and its parents) readable only by the user (B1).
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}
