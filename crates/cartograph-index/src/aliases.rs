//! Host aliases (XS-02, C6, C7): the names a service answers to on the network, with intervals.
//!
//! The `manifest-deploy` extractor reads each compose file and k8s manifest blob once and says the
//! hosts in it as facts: compose service names, `container_name` and `networks.*.aliases`; a k8s
//! `Service`'s `name`, `name.ns`, `name.ns.svc` and `name.ns.svc.cluster.local`. Here those facts
//! become rows: each version of each such file gives its hosts an interval, the time that version
//! was at a path (`file_path` and `file_version`, as for every other row of the index). A row
//! names the blob it came from (`blob_id`) and the file (`source`).
//!
//! A host reaches the service whose root holds the folder it is built from (a compose `build`
//! context, resolved against the file's folder) or sits in (a k8s manifest's folder), the deepest
//! root winning. A `build` that is absolute, a URL or climbs out of the repository reaches nothing
//! (XS-02 security).
//!
//! The files are tracked like any other, found by the walk from fixed names; nothing is scanned
//! and nothing is run. Rows are rebuilt only when the facts or the service nodes they map to
//! changed (the `host_alias` derived key).

use std::collections::BTreeMap;

use cartograph_parse::{BlobFacts, Fact};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, params};

use crate::derive::key;
use crate::structure::{dir, merge, under};

/// Bump when what the rows hold changes, so every index rebuilds them.
const ALIAS_VERSION: &str = "host_alias 1";

/// `d/rest`, normalised; `None` when `rest` is absolute, a URL, or climbs out of the repository.
pub(crate) fn inside(d: &str, rest: &str) -> Option<String> {
    if rest.starts_with('/') || rest.starts_with('\\') || rest.contains("://") {
        return None;
    }
    let mut parts: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// One version of a deploy file at a path: the overlap of the two intervals, and its facts.
struct Version {
    /// As in `file_path` (with the repository's prefix in a multi-repository index).
    path: String,
    /// Repository-relative.
    rel: String,
    blob: Vec<u8>,
    from: i64,
    to: Option<i64>,
    payload: Vec<u8>,
}

fn versions(
    conn: &Connection,
    repo_id: i64,
    prefix: Option<&str>,
) -> rusqlite::Result<Vec<Version>> {
    let mut st = conn.prepare_cached(
        "SELECT fp.path, fp.from_seq, fp.to_seq, fv.blob, fv.from_seq, fv.to_seq, fa.payload
         FROM file_path fp JOIN file f ON f.id = fp.file
           JOIN file_version fv ON fv.file = fp.file
           JOIN fact fa ON fa.blob = fv.blob
           JOIN extractor e ON e.id = fa.extractor
         WHERE f.repo = ?1 AND e.name LIKE 'manifest-deploy/%'
         ORDER BY fp.path, fp.from_seq, fv.from_seq, fv.blob",
    )?;
    let mut rows = st.query([repo_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let path: String = r.get(0)?;
        let from = r.get::<_, i64>(1)?.max(r.get(4)?);
        let to = match (r.get::<_, Option<i64>>(2)?, r.get::<_, Option<i64>>(5)?) {
            (None, t) | (t, None) => t,
            (Some(a), Some(b)) => Some(a.min(b)),
        };
        if to.is_some_and(|t| t <= from) {
            continue;
        }
        let rel = match prefix {
            Some(x) => path
                .strip_prefix(x)
                .map_or(path.as_str(), |r| r.trim_start_matches('/')),
            None => &path,
        }
        .to_owned();
        out.push(Version {
            path,
            rel,
            blob: r.get(3)?,
            from,
            to,
            payload: r.get(6)?,
        });
    }
    Ok(out)
}

/// Rebuilds one repository's `host_alias` rows when the deploy facts or the service nodes they map
/// to changed since the last run. Returns the rows written. Aliases of a service node no longer
/// detected go with the rows: they are rewritten against the services found now.
pub(crate) fn refresh(
    conn: &Connection,
    repo_id: i64,
    repo_name: &str,
    prefix: Option<&str>,
    config: Option<&str>,
) -> rusqlite::Result<usize> {
    let key_lo = format!("service:{repo_name}:");
    let key_hi = format!("service:{repo_name};");
    // root path -> node, for the services found now.
    let nodes: BTreeMap<String, i64> = conn
        .prepare_cached(
            "SELECT key, id FROM node WHERE key >= ?1 AND key < ?2 AND detected_by IS NOT NULL
             ORDER BY key",
        )?
        .query_map(params![key_lo, key_hi], |r| {
            Ok((r.get::<_, String>(0)?[key_lo.len()..].to_owned(), r.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    let versions = versions(conn, repo_id, prefix)?;

    let mut inputs: Vec<Vec<u8>> = nodes
        .iter()
        .map(|(p, id)| format!("node\0{p}\0{id}").into_bytes())
        .collect();
    inputs.extend(versions.iter().map(|v| {
        let mut b = format!("{}\0{}\0{:?}\0", v.path, v.from, v.to).into_bytes();
        b.extend_from_slice(&v.blob);
        b.extend_from_slice(&v.payload);
        b
    }));
    let k = key("host_alias", ALIAS_VERSION, inputs, config);
    if !changed(conn, "host_alias", repo_name, &k)? {
        return Ok(0);
    }

    conn.execute(
        "DELETE FROM host_alias WHERE service_id IN
           (SELECT id FROM node WHERE key >= ?1 AND key < ?2)",
        params![key_lo, key_hi],
    )?;
    // One row per (host, port, service, file, blob), its spans merged.
    type Row = (String, Option<u16>, i64, String, Vec<u8>);
    let mut rows: BTreeMap<Row, Vec<(i64, Option<i64>)>> = BTreeMap::new();
    for v in &versions {
        let Some(BlobFacts::Facts(facts)) = BlobFacts::decode(&v.payload) else {
            continue;
        };
        for fact in facts {
            let Fact::Host {
                host,
                port,
                context,
            } = fact
            else {
                continue;
            };
            let folder = dir(&v.rel);
            let at = match &context {
                Some(c) => inside(folder, c),
                None => Some(folder.to_owned()),
            };
            // The deepest service root that holds the folder.
            let Some((_, &service)) = at.and_then(|at| {
                nodes
                    .iter()
                    .filter(|(root, _)| under(root, &at))
                    .max_by_key(|(root, _)| root.len())
            }) else {
                continue;
            };
            rows.entry((host, port, service, v.path.clone(), v.blob.clone()))
                .or_default()
                .push((v.from, v.to));
        }
    }
    let mut written = 0;
    for ((host, port, service, source, blob), spans) in rows {
        for (from, to) in merge(spans) {
            conn.prepare_cached(
                "INSERT INTO host_alias (host, port, env_id, service_id, source, blob_id,
                   valid_from, valid_to) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![host, port, service, source, blob, from, to])?;
            written += 1;
        }
    }
    record(conn, "host_alias", repo_name, &k)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_context_that_leaves_the_repository_reaches_nothing() {
        for ctx in [
            "/etc",
            "\\x",
            "../outside",
            "https://example.com/x.git",
            "a/../../b",
        ] {
            assert_eq!(inside("", ctx), None, "{ctx}");
        }
        assert_eq!(inside("apps", "../shared/x").as_deref(), Some("shared/x"));
        assert_eq!(inside("apps", "./api/").as_deref(), Some("apps/api"));
        assert_eq!(inside("", ".").as_deref(), Some(""));
    }
}
