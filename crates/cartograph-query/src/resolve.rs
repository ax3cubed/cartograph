//! Turning what a caller wrote into rows: `at=` into a timeline step, a node reference into a node.

use cartograph_api::types::CommitRef;
use cartograph_store::rusqlite::OptionalExtension;

use crate::{At, QueryError, Snapshot};

/// How many candidates an ambiguous reference lists.
const CANDIDATES: usize = 10;

/// The wire id of node row `id`.
// ponytail: ids are row ids, stable for one index file but not across a rebuild. Content-derived
// ids arrive with rename lineage (IDX-18); until then a client re-reads ids after a rebuild.
pub fn node_id(id: i64) -> String {
    format!("n{id}")
}

pub fn link_id(id: i64) -> String {
    format!("l{id}")
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    s.len()
        .is_multiple_of(2)
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

fn no_commit() -> QueryError {
    QueryError::NotFound {
        what: "no indexed commit matches at".into(),
        action: "pass a commit on the indexed branch, a later date, or now".into(),
    }
}

impl Snapshot<'_> {
    /// The step `at` names: the commit itself, or the last one at or before a date.
    pub fn resolve_at(&self, at: &At) -> Result<CommitRef, QueryError> {
        let c = self.conn();
        let row = |r: &cartograph_store::rusqlite::Row<'_>| -> cartograph_store::rusqlite::Result<CommitRef> {
            Ok(CommitRef {
                seq: r.get::<_, i64>(0)? as u64,
                sha: hex(&r.get::<_, Vec<u8>>(1)?),
            })
        };
        let found = match at {
            At::Now => c
                .prepare_cached("SELECT seq, sha FROM commits ORDER BY seq DESC LIMIT 1")?
                .query_row([], row)
                .optional()?,
            At::Time(t) => c
                .prepare_cached(
                    "SELECT seq, sha FROM commits WHERE corrected_time <= ?1
                     ORDER BY corrected_time DESC LIMIT 1",
                )?
                .query_row([t], row)
                .optional()?,
            At::Sha(s) if matches!(s.len(), 40 | 64) => c
                .prepare_cached("SELECT seq, sha FROM commits WHERE sha = ?1 ORDER BY seq LIMIT 1")?
                .query_row([parse_hex(s).ok_or_else(no_commit)?], row)
                .optional()?,
            At::Sha(prefix) => {
                // ponytail: a short sha scans the commits table (60k rows at most); a blob range
                // on the sha index would make it a seek.
                let mut st = c.prepare_cached(
                    "SELECT MIN(seq), sha FROM commits WHERE substr(hex(sha), 1, length(?1)) = upper(?1)
                     GROUP BY sha ORDER BY 1 LIMIT 11",
                )?;
                let hits: Vec<CommitRef> =
                    st.query_map([prefix], row)?.collect::<Result<_, _>>()?;
                if hits.len() > 1 {
                    return Err(QueryError::Ambiguous {
                        reference: prefix.clone(),
                        candidates: hits.into_iter().take(CANDIDATES).map(|h| h.sha).collect(),
                    });
                }
                hits.into_iter().next()
            }
        };
        found.ok_or_else(no_commit)
    }

    /// A node from an id (`n123`), `repo:path`, a path, or a file name unique at `seq`.
    /// An ambiguous name returns its candidates, never a guess.
    pub fn resolve_node(&self, reference: &str, seq: u64) -> Result<String, QueryError> {
        if reference.is_empty() || reference.len() > 1024 || reference.chars().any(char::is_control)
        {
            return Err(QueryError::bad(
                "the node reference is malformed",
                "a reference is an id, repo:path or a name, up to 1,024 printable characters",
                "find the node with search, then pass its id",
            ));
        }
        let c = self.conn();
        let seq = seq as i64;
        if let Some(n) = reference
            .strip_prefix('n')
            .filter(|d| (1..=18).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse::<i64>().ok())
        {
            let hit = c
                .prepare_cached("SELECT id FROM node WHERE id = ?1")?
                .query_row([n], |r| r.get::<_, i64>(0))
                .optional()?;
            if let Some(id) = hit {
                return Ok(node_id(id));
            }
        }
        let row = |r: &cartograph_store::rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?));
        let hits: Vec<(i64, String)> = match reference.split_once(':') {
            Some((repo, path)) => c
                .prepare_cached(
                    // A multi-repo index stores `repo/path` (one repo keeps bare paths), and the
                    // repo:path form names the path inside the repo either way.
                    "SELECT n.id, fp.path FROM file_path fp JOIN node n ON n.file = fp.file
                     JOIN file f ON f.id = fp.file JOIN repo r ON r.id = f.repo
                     WHERE fp.from_seq <= ?1 AND (fp.to_seq IS NULL OR fp.to_seq > ?1)
                       AND (fp.path = ?2 OR fp.path = r.name || '/' || ?2) AND r.name = ?3
                     ORDER BY fp.path LIMIT 11",
                )?
                .query_map((seq, path, repo), row)?
                .collect::<Result<_, _>>()?,
            None => c
                .prepare_cached(
                    "SELECT n.id, fp.path FROM file_path fp JOIN node n ON n.file = fp.file
                     WHERE fp.from_seq <= ?1 AND (fp.to_seq IS NULL OR fp.to_seq > ?1)
                       AND (fp.path = ?2 OR substr(fp.path, -length(?2) - 1) = '/' || ?2)
                     ORDER BY fp.path LIMIT 11",
                )?
                .query_map((seq, reference), row)?
                .collect::<Result<_, _>>()?,
        };
        // No file: a service or module by its name, its folder or its key.
        let hits = if hits.is_empty() {
            c.prepare_cached(
                "SELECT n.id, n.key FROM node n WHERE n.kind != 'file' AND n.key IS NOT NULL
                   AND (n.name = ?1 OR n.key = 'module:' || ?1
                        OR (n.key LIKE 'service:%' AND substr(n.key, -length(?1) - 1) = ':' || ?1))
                 ORDER BY n.key LIMIT 11",
            )?
            .query_map([reference], row)?
            .collect::<Result<_, _>>()?
        } else {
            hits
        };
        match hits.as_slice() {
            [] => Err(QueryError::NotFound {
                what: "no node has that name".into(),
                action: "check the spelling, or search for it".into(),
            }),
            [(id, _)] => Ok(node_id(*id)),
            _ => Err(QueryError::Ambiguous {
                reference: reference.into(),
                candidates: hits
                    .iter()
                    .take(CANDIDATES)
                    .map(|(id, p)| format!("{} {p}", node_id(*id)))
                    .collect(),
            }),
        }
    }
}
