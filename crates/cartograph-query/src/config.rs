//! Environments and the config behind a link (XS-09): `GET …/environments`, and the chain a link's
//! call is resolved through (`?env=` on `GET …/links/{id}`).

use std::collections::BTreeMap;

use cartograph_api::types::{Environment, LinkConfig};
use cartograph_core::secrets::mask;
use cartograph_store::rusqlite::{self, OptionalExtension, params};

use crate::{QueryError, Snapshot};

/// The host of a value: `http://catalog.prod.svc:8080/api` is `catalog.prod.svc:8080`, and
/// `catalog:8080` is itself.
fn host_of(value: &str) -> String {
    let rest = value.split_once("://").map_or(value, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_owned()
}

/// What `link_env` says about one link.
pub(crate) struct LinkEnv {
    pub env: Option<String>,
    pub envs: Vec<String>,
    pub config: Vec<LinkConfig>,
}

impl Snapshot<'_> {
    /// `GET …/environments`: each environment, the default marked, with the locator values it has
    /// as of the newest commit. An environment's own value wins over one every environment shares.
    pub fn environments(&self) -> Result<Vec<Environment>, QueryError> {
        let mut out: Vec<(i64, Environment)> = self
            .conn()
            .prepare_cached("SELECT id, name, is_default FROM environment ORDER BY id")?
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    Environment {
                        name: r.get(1)?,
                        is_default: r.get::<_, i64>(2)? != 0,
                        values: BTreeMap::new(),
                    },
                ))
            })?
            .collect::<Result<_, _>>()?;
        let mut st = self.conn().prepare_cached(
            "SELECT key, env_id, value FROM config_value
             WHERE valid_to IS NULL AND value IS NOT NULL
             ORDER BY env_id IS NOT NULL, key, source",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let (key, env, value): (String, Option<i64>, String) =
                (r.get(0)?, r.get(1)?, r.get(2)?);
            for (id, e) in &mut out {
                if env.is_none() || env == Some(*id) {
                    // Shared rows come first, so an environment's own replaces them.
                    e.values.insert(key.clone(), mask(&value).into_owned());
                }
            }
        }
        Ok(out.into_iter().map(|(_, e)| e).collect())
    }

    /// The bit of an environment in a link's `envs`; a name the workspace doesn't have is a bad
    /// request.
    pub(crate) fn env_bit(&self, name: &str) -> Result<u32, QueryError> {
        let id: Option<i64> = self
            .conn()
            .prepare_cached("SELECT id FROM environment WHERE name = ?1")?
            .query_row([name], |r| r.get(0))
            .optional()?;
        id.map(|i| 1u32 << i).ok_or_else(|| {
            QueryError::bad(
                format!("no environment named {name}"),
                "the workspace has no environment of that name",
                "take the name from GET …/environments",
            )
        })
    }

    /// The environments an HTTP link holds in, and how `env` (the default when `None`) resolves
    /// it at `seq`. A link that depends on no environment has an empty answer; a name the
    /// workspace doesn't have is a bad request.
    pub(crate) fn link_env(
        &self,
        link: i64,
        seq: i64,
        env: Option<&str>,
    ) -> Result<LinkEnv, QueryError> {
        let envs: Vec<(i64, String, bool)> = self
            .conn()
            .prepare_cached("SELECT id, name, is_default FROM environment ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?
            .collect::<Result<_, _>>()?;
        let chosen = match env {
            Some(name) => Some(envs.iter().find(|e| e.1 == name).ok_or_else(|| {
                QueryError::bad(
                    format!("no environment named {name}"),
                    format!(
                        "the workspace has {}",
                        envs.iter()
                            .map(|e| e.1.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    "take the name from GET …/environments",
                )
            })?),
            None => envs.iter().find(|e| e.2),
        };
        let mask_bits: Option<i64> = self
            .conn()
            .prepare_cached("SELECT envs FROM link WHERE id = ?1")?
            .query_row([link], |r| r.get(0))
            .optional()?
            .flatten();
        let Some(bits) = mask_bits else {
            return Ok(LinkEnv {
                env: None,
                envs: vec![],
                config: vec![],
            });
        };
        let holds: Vec<String> = envs
            .iter()
            .filter(|e| bits & (1 << e.0) != 0)
            .map(|e| e.1.clone())
            .collect();
        let Some((id, name, _)) = chosen else {
            return Ok(LinkEnv {
                env: None,
                envs: holds,
                config: vec![],
            });
        };
        // What held at `seq`, else what held last before it.
        let mut st = self.conn().prepare_cached(
            "SELECT key, value, source FROM link_config
             WHERE link = ?1 AND env_id = ?2 AND from_seq <= ?3 AND (to_seq IS NULL OR to_seq > ?3)
             ORDER BY from_seq, key",
        )?;
        let mut config: Vec<LinkConfig> = st
            .query_map(params![link, id, seq], to_config)?
            .collect::<Result<_, _>>()?;
        if config.is_empty() {
            config = self
                .conn()
                .prepare_cached(
                    "SELECT key, value, source FROM link_config
                     WHERE link = ?1 AND env_id = ?2 AND from_seq <= ?3
                       AND from_seq = (SELECT max(from_seq) FROM link_config
                                       WHERE link = ?1 AND env_id = ?2 AND from_seq <= ?3)
                     ORDER BY key",
                )?
                .query_map(params![link, id, seq], to_config)?
                .collect::<Result<_, _>>()?;
        }
        Ok(LinkEnv {
            env: Some(name.clone()),
            envs: holds,
            config,
        })
    }
}

fn to_config(r: &rusqlite::Row<'_>) -> rusqlite::Result<LinkConfig> {
    let value: String = r.get(1)?;
    let value = mask(&value).into_owned();
    Ok(LinkConfig {
        key: r.get(0)?,
        host: host_of(&value),
        value,
        source: r.get(2)?,
    })
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn the_host_is_what_the_value_names() {
        assert_eq!(
            host_of("http://catalog.prod.svc:8080/api"),
            "catalog.prod.svc:8080"
        );
        assert_eq!(host_of("catalog:8080"), "catalog:8080");
        assert_eq!(host_of("https://x.test"), "x.test");
    }
}
