//! Host resolution (XS-02): which service a URL or `host:port` in code or config reaches, by the
//! host aliases the index holds (compose service names, container names, network aliases, k8s
//! Service names), as they stood at a step.

use cartograph_core::NodeId;
use cartograph_store::rusqlite::params;

use crate::{At, QueryError, Snapshot, node_id};

/// The service a host name reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMatch {
    pub service: NodeId,
    /// The service's name.
    pub name: String,
    /// The alias that matched, lower-cased.
    pub host: String,
    /// The port that alias declares, if it declares one.
    pub port: Option<u16>,
    /// The file that declares it.
    pub source: String,
}

/// The host and port of `http://orders:8080/x`, `orders:8080`, `orders.shop.svc`: any scheme,
/// credentials, path, query and fragment dropped, the host lower-cased without a trailing dot.
/// `None` for an empty host, a bracketed (IPv6) host or a port that isn't a number.
pub fn host_and_port(target: &str) -> Option<(String, Option<u16>)> {
    let t = target.trim();
    let t = t.split_once("://").map_or(t, |(_, r)| r);
    let t = t.split(['/', '?', '#']).next().unwrap_or("");
    let t = t.rsplit_once('@').map_or(t, |(_, h)| h);
    if t.starts_with('[') {
        return None;
    }
    let (host, port) = match t.rsplit_once(':') {
        Some((h, p)) => (h, Some(p.parse::<u16>().ok()?)),
        None => (t, None),
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some((host, port))
}

impl Snapshot<'_> {
    /// The service `target` (a URL, or `host`, or `host:port`) reaches at `at`, if one alias
    /// does. With a port, an alias that declares that port beats one that declares none, and one
    /// that declares another doesn't match. Two services tied for the best match is no match: a
    /// wrong answer would read as a link.
    pub fn resolve_host(&self, target: &str, at: &At) -> Result<Option<HostMatch>, QueryError> {
        let Some((host, port)) = host_and_port(target) else {
            return Ok(None);
        };
        let seq = self.resolve_at(at)?.seq as i64;
        let mut st = self.conn().prepare_cached(
            "SELECT n.id, n.name, h.host, h.port, h.source, (h.port IS NOT NULL) FROM host_alias h
               JOIN node n ON n.id = h.service_id
             WHERE h.host = ?1 AND (?2 IS NULL OR h.port IS NULL OR h.port = ?2)
               AND h.valid_from <= ?3 AND (h.valid_to IS NULL OR h.valid_to > ?3)
             ORDER BY (h.port IS NOT NULL) DESC, n.id, h.source",
        )?;
        let rows = st.query_map(params![host, port, seq], |r| {
            let hit = HostMatch {
                service: NodeId(node_id(r.get(0)?)),
                name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                host: r.get(2)?,
                port: r.get(3)?,
                source: r.get(4)?,
            };
            Ok((hit, r.get::<_, bool>(5)?))
        })?;
        let mut best: Option<(HostMatch, bool)> = None;
        for row in rows {
            let row = row?;
            match &best {
                None => best = Some(row),
                // Rows run best first: a lower rank, or another service at the same rank, ends it.
                Some((_, rank)) if *rank != row.1 => break,
                Some((b, _)) if b.service != row.0.service => return Ok(None),
                Some(_) => {}
            }
        }
        Ok(best.map(|(hit, _)| hit))
    }
}
