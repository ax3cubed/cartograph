//! Config values and environments (XS-09): what each environment sets a key to, with intervals.
//!
//! The `manifest-deploy` extractor reads each `.env.example`, compose file, k8s manifest, Helm
//! values file and Helm template once and says the keys and locator values in it as facts (see
//! `cartograph_parse::config`). Here those facts become rows: each version of each file gives its
//! keys an interval, the time that version was at a path, like every other row of the index.
//!
//! The environment of a file is the one a manifest `environments.<name>.sources` glob names, else
//! the one its name implies: `values-prod.yaml`, `.env.staging.example`, `overlays/staging/`;
//! compose files and `.env.example` are `local`. A file that implies none gives every environment
//! its values, which an environment's own file overrides. A Helm template's `{{ .Values.a.b }}`
//! takes the value of `a.b` in the values files of its own chart.
//!
//! Values the manifest sets by hand (`environments.<name>.set`) hold for the whole history. A
//! value is stored only when it classifies as a locator (XS-10).

use std::collections::{BTreeMap, BTreeSet};

use cartograph_core::secrets::{ConfigValue, classify_config_value};
use cartograph_parse::{ConfigScope, ConfigVal, Fact};
use cartograph_store::derived::{changed, record};
use cartograph_store::rusqlite::{self, Connection, params};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::derive::key;
use crate::structure::{dir, merge};

/// Bump when what the rows hold changes, so every index rebuilds them.
const CONFIG_VERSION: &str = "config 1";
/// The bitmask of a link holds this many environments.
pub const MAX_ENVS: usize = 32;

/// One entry of the manifest's `environments:`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSpec {
    pub name: String,
    pub sources: Vec<String>,
    pub set: BTreeMap<String, String>,
}

/// An environment as the index knows it: `id` is its bit in a link's `envs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub id: i64,
    pub name: String,
}

/// The environments of the index, by id.
pub fn environments(conn: &Connection) -> rusqlite::Result<Vec<Env>> {
    conn.prepare_cached("SELECT id, name FROM environment ORDER BY id")?
        .query_map([], |r| {
            Ok(Env {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect()
}

/// One version of a value-source file at a path: the overlap of the two intervals.
struct Version {
    path: String,
    blob: Vec<u8>,
    from: i64,
    to: Option<i64>,
    payload: Vec<u8>,
}

fn versions(conn: &Connection, payload: bool) -> rusqlite::Result<Vec<Version>> {
    let mut st = conn.prepare_cached(
        "SELECT fp.path, fp.from_seq, fp.to_seq, fv.blob, fv.from_seq, fv.to_seq,
                CASE WHEN ?1 THEN fa.payload ELSE e.version END
         FROM file_path fp JOIN file_version fv ON fv.file = fp.file
           JOIN fact fa ON fa.blob = fv.blob
           JOIN extractor e ON e.id = fa.extractor
         WHERE e.name LIKE 'manifest-deploy/%'
         ORDER BY fp.path, fp.from_seq, fv.from_seq, fv.blob",
    )?;
    let mut rows = st.query([payload])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let from = r.get::<_, i64>(1)?.max(r.get(4)?);
        let to = match (r.get::<_, Option<i64>>(2)?, r.get::<_, Option<i64>>(5)?) {
            (None, t) | (t, None) => t,
            (Some(a), Some(b)) => Some(a.min(b)),
        };
        if to.is_some_and(|t| t <= from) {
            continue;
        }
        out.push(Version {
            path: r.get(0)?,
            blob: r.get(3)?,
            from,
            to,
            payload: if payload {
                r.get(6)?
            } else {
                r.get::<_, String>(6)?.into_bytes()
            },
        });
    }
    Ok(out)
}

/// `production` is `prod`; names a file doesn't mean as an environment are none.
fn norm(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "" | "base" | "default" | "common" | "all" | "values" => None,
        "production" => Some("prod".into()),
        _ => Some(n),
    }
}

/// The environment a path implies, or `None` when it implies none (every environment).
pub fn infer(path: &str) -> Option<String> {
    let b = path.rsplit('/').next().unwrap_or(path);
    if cartograph_parse::config::is_dotenv_example(path) {
        // `.env.staging.example` is staging; `.env.example` is local.
        let mid = b.trim_start_matches(".env");
        let mid = mid.rsplit_once('.').map_or("", |(m, _)| m);
        return match mid.trim_start_matches('.') {
            "" => Some("local".into()),
            n => norm(n),
        };
    }
    if cartograph_parse::deploy::is_compose_name(path) {
        return Some("local".into());
    }
    let segs: Vec<&str> = path.split('/').collect();
    for w in segs[..segs.len() - 1].windows(2) {
        if matches!(w[0], "overlays" | "environments" | "envs" | "env") {
            return norm(w[1]);
        }
    }
    if cartograph_parse::config::is_values_name(path) {
        let stem = b.rsplit_once('.').map_or(b, |(s, _)| s);
        return norm(
            stem.trim_start_matches("values")
                .trim_start_matches(['-', '.', '_']),
        );
    }
    None
}

fn sources(specs: &[EnvSpec]) -> Vec<(String, GlobSet)> {
    specs
        .iter()
        .filter(|s| !s.sources.is_empty())
        .map(|s| {
            let mut b = GlobSetBuilder::new();
            for g in &s.sources {
                if let Ok(g) = Glob::new(g) {
                    b.add(g);
                }
            }
            (
                s.name.clone(),
                b.build().unwrap_or_else(|_| GlobSet::empty()),
            )
        })
        .collect()
}

/// `(a_from, a_to)` and `(b_from, b_to)` overlapped.
fn overlap(a: (i64, Option<i64>), b: (i64, Option<i64>)) -> Option<(i64, Option<i64>)> {
    let from = a.0.max(b.0);
    let to = match (a.1, b.1) {
        (None, t) | (t, None) => t,
        (Some(x), Some(y)) => Some(x.min(y)),
    };
    to.is_none_or(|t| t > from).then_some((from, to))
}

/// The chart a template belongs to: the folder that holds its `templates` folder.
fn chart_of(path: &str) -> Option<&str> {
    if path.starts_with("templates/") {
        return Some("");
    }
    path.rfind("/templates/").map(|i| &path[..i])
}

type RowKey = (String, Option<String>, Option<String>, String, Vec<u8>);

/// Rebuilds the environments and config values when a value-source file, an `environments:` entry
/// or `default_environment` changed. Returns whether it did.
pub(crate) fn refresh(
    conn: &Connection,
    specs: &[EnvSpec],
    default: Option<&str>,
) -> rusqlite::Result<bool> {
    let mut inputs: Vec<Vec<u8>> = versions(conn, false)?
        .iter()
        .map(|v| {
            let mut b = format!("{}\0{}\0{:?}\0", v.path, v.from, v.to).into_bytes();
            b.extend_from_slice(&v.blob);
            b.extend_from_slice(&v.payload);
            b
        })
        .collect();
    for s in specs {
        inputs.push(format!("env\0{}\0{:?}\0{:?}", s.name, s.sources, s.set).into_bytes());
    }
    inputs.push(format!("default\0{default:?}").into_bytes());
    let k = key("config", CONFIG_VERSION, inputs, None);
    if !changed(conn, "config", "workspace", &k)? {
        return Ok(false);
    }

    let globs = sources(specs);
    let env_of = |path: &str| -> Option<String> {
        globs
            .iter()
            .find(|(_, g)| g.is_match(path))
            .map(|(n, _)| n.clone())
            .or_else(|| infer(path))
    };
    // (env, key, locator or opaque, source, blob) -> intervals.
    let mut rows: BTreeMap<RowKey, Vec<(i64, Option<i64>)>> = BTreeMap::new();
    // A chart's values: (chart folder) -> (env, dotted key, value, source, blob, from, to).
    type Val = (
        Option<String>,
        String,
        Option<String>,
        String,
        Vec<u8>,
        i64,
        Option<i64>,
    );
    let mut values: BTreeMap<String, Vec<Val>> = BTreeMap::new();
    // (chart folder, NAME, dotted key, from, to)
    let mut refs: Vec<(String, String, String, i64, Option<i64>)> = Vec::new();
    for v in versions(conn, true)? {
        let Some(cartograph_parse::BlobFacts::Facts(facts)) =
            cartograph_parse::BlobFacts::decode(&v.payload)
        else {
            continue;
        };
        let env = env_of(&v.path);
        for f in facts {
            let Fact::Config { scope, key, value } = f else {
                continue;
            };
            match (scope, value) {
                (ConfigScope::Values, value) => {
                    let value = match value {
                        ConfigVal::Locator(l) => Some(l),
                        _ => None,
                    };
                    values.entry(dir(&v.path).to_owned()).or_default().push((
                        env.clone(),
                        key,
                        value,
                        v.path.clone(),
                        v.blob.clone(),
                        v.from,
                        v.to,
                    ));
                }
                (ConfigScope::Env, ConfigVal::Ref(r)) => {
                    if let Some(c) = chart_of(&v.path) {
                        refs.push((c.to_owned(), key, r, v.from, v.to));
                    }
                }
                (ConfigScope::Env, value) => {
                    let value = match value {
                        ConfigVal::Locator(l) => Some(l),
                        _ => None,
                    };
                    rows.entry((key, env.clone(), value, v.path.clone(), v.blob.clone()))
                        .or_default()
                        .push((v.from, v.to));
                }
            }
        }
    }
    for (chart, name, r, from, to) in refs {
        for (env, key, value, source, blob, vf, vt) in values.get(&chart).into_iter().flatten() {
            if *key != r {
                continue;
            }
            if let Some((f, t)) = overlap((from, to), (*vf, *vt)) {
                rows.entry((
                    name.clone(),
                    env.clone(),
                    value.clone(),
                    source.clone(),
                    blob.clone(),
                ))
                .or_default()
                .push((f, t));
            }
        }
    }
    // What the manifest sets by hand holds for the whole history.
    for s in specs {
        for (key, v) in &s.set {
            let value = match classify_config_value(key, v) {
                ConfigValue::Locator(l) => Some(l),
                ConfigValue::Opaque => None,
            };
            rows.entry((
                key.clone(),
                Some(s.name.clone()),
                value,
                "cartograph.yaml".into(),
                Vec::new(),
            ))
            .or_default()
            .push((0, None));
        }
    }

    // The environments: declared and inferred, by name; ids are positions.
    let mut names: BTreeSet<String> = specs.iter().map(|s| s.name.clone()).collect();
    names.extend(rows.keys().filter_map(|k| k.1.clone()));
    names.extend(values.values().flatten().filter_map(|v| v.0.clone()));
    let names: Vec<String> = names.into_iter().take(MAX_ENVS).collect();
    let default = default
        .filter(|d| names.iter().any(|n| n == d))
        .or_else(|| names.iter().find(|n| *n == "prod").map(String::as_str))
        .or_else(|| names.first().map(String::as_str));

    conn.execute("DELETE FROM config_value", [])?;
    let same = environments(conn)?
        .iter()
        .map(|e| e.name.as_str())
        .eq(names.iter().map(String::as_str));
    // Links resolved through an environment that is renumbered or gone are rebuilt by the HTTP
    // pass, whose key holds the environments.
    if !same {
        conn.execute_batch("DELETE FROM link_config; DELETE FROM environment;")?;
    }
    for (i, n) in names.iter().enumerate() {
        conn.prepare_cached(
            "INSERT INTO environment (id, name, is_default) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET is_default = excluded.is_default",
        )?
        .execute(params![i as i64, n, i64::from(Some(n.as_str()) == default)])?;
    }
    let id_of = |n: &Option<String>| -> Option<Option<i64>> {
        match n {
            None => Some(None),
            Some(n) => names.iter().position(|x| x == n).map(|i| Some(i as i64)),
        }
    };
    for ((key, env, value, source, blob), spans) in rows {
        let Some(env_id) = id_of(&env) else { continue };
        for (from, to) in merge(spans) {
            conn.prepare_cached(
                "INSERT INTO config_value (key, env_id, value, value_kind, source, blob_id,
                   valid_from, valid_to) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                key,
                env_id,
                value,
                if value.is_some() { "locator" } else { "opaque" },
                source,
                blob,
                from,
                to
            ])?;
        }
    }
    record(conn, "config", "workspace", &k)?;
    Ok(true)
}

/// One stretch of a value: `None` is no value.
pub(crate) type Stretch = (i64, Option<i64>, Option<(String, String)>);

/// What `key` is in environment `env` over `within`, as stretches that cover it: the
/// environment's own rows win over the base ones, and an opaque row is no value. Each value is
/// `(locator, source)`.
pub(crate) fn effective(
    own: &[(String, Option<String>, i64, Option<i64>)],
    base: &[(String, Option<String>, i64, Option<i64>)],
    within: (i64, Option<i64>),
) -> Vec<Stretch> {
    let mut cuts: BTreeSet<i64> = BTreeSet::from([within.0]);
    for (_, _, f, t) in own.iter().chain(base) {
        cuts.extend([*f].into_iter().chain(*t));
    }
    let cuts: Vec<i64> = cuts
        .into_iter()
        .filter(|c| *c >= within.0 && within.1.is_none_or(|e| *c < e))
        .collect();
    let live = |rows: &[(String, Option<String>, i64, Option<i64>)], at: i64| {
        rows.iter()
            .filter(|(_, _, f, t)| *f <= at && t.is_none_or(|t| at < t))
            .min_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)))
            .map(|(s, v, _, _)| (s.clone(), v.clone()))
    };
    let mut out: Vec<Stretch> = Vec::new();
    for (i, &at) in cuts.iter().enumerate() {
        let end = cuts.get(i + 1).copied().or(within.1);
        let val = live(own, at)
            .or_else(|| live(base, at))
            .and_then(|(source, v)| v.map(|v| (v, source)));
        match out.last_mut() {
            Some(last) if last.1 == Some(at) && last.2 == val => last.1 = end,
            _ => out.push((at, end, val)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_is_named_by_the_file() {
        for (p, want) in [
            ("charts/catalog/values-prod.yaml", Some("prod")),
            ("charts/catalog/values-production.yaml", Some("prod")),
            ("charts/catalog/values.staging.yml", Some("staging")),
            ("charts/catalog/values.yaml", None),
            (".env.example", Some("local")),
            ("web/.env.staging.example", Some("staging")),
            ("docker-compose.yml", Some("local")),
            ("deploy/overlays/staging/cm.yaml", Some("staging")),
            ("deploy/base/cm.yaml", None),
        ] {
            assert_eq!(infer(p).as_deref(), want, "{p}");
        }
    }

    fn row(
        s: &str,
        v: Option<&str>,
        f: i64,
        t: Option<i64>,
    ) -> (String, Option<String>, i64, Option<i64>) {
        (s.into(), v.map(Into::into), f, t)
    }

    #[test]
    fn an_environments_own_value_wins_and_a_gap_is_no_value() {
        let own = [row("prod.yaml", Some("http://p"), 2, Some(5))];
        let base = [row("values.yaml", Some("http://b"), 0, None)];
        let got = effective(&own, &base, (0, None));
        let want: Vec<Stretch> = vec![
            (0, Some(2), Some(("http://b".into(), "values.yaml".into()))),
            (2, Some(5), Some(("http://p".into(), "prod.yaml".into()))),
            (5, None, Some(("http://b".into(), "values.yaml".into()))),
        ];
        assert_eq!(got, want);
        // Removing the value at 5 leaves a stretch with none.
        let base = [row("values.yaml", Some("http://b"), 0, Some(5))];
        let got = effective(&[], &base, (1, None));
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], (5, None, None));
        // An opaque own row hides the base value.
        let own = [row("p", None, 0, None)];
        assert_eq!(effective(&own, &base, (0, None))[0].2, None);
    }
}
