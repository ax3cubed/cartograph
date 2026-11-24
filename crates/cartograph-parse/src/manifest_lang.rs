//! manifest-go, manifest-py and manifest-php (IDX-23): the fields of go.mod, go.work,
//! pyproject.toml, setup.cfg and composer.json that resolution reads, each as canonical JSON. They
//! are the resolvers' inputs (content keys), as package.json's fields are for TypeScript. Parsed
//! as data with the blob size cap; nothing is run.

use serde_json::{Value, json};

use crate::{BlobFacts, Extractor, Fact};

pub struct ManifestGo;
pub struct ManifestPy;
pub struct ManifestPhp;

const GO_MOD: &str = "manifest-go/mod";
const GO_WORK: &str = "manifest-go/work";
const PYPROJECT: &str = "manifest-py/pyproject";
const SETUP_CFG: &str = "manifest-py/setup";
const COMPOSER: &str = "manifest-php/composer";

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn facts(fields: Vec<(&str, Value)>) -> BlobFacts {
    let mut f: Vec<Fact> = fields
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| Fact::Manifest {
            field: k.to_owned(),
            value: v.to_string(),
        })
        .collect();
    f.sort();
    BlobFacts::Facts(f)
}

fn text(bytes: &[u8]) -> Result<&str, BlobFacts> {
    if let Some(r) = crate::skip_reason(bytes) {
        return Err(BlobFacts::Skipped(r));
    }
    std::str::from_utf8(bytes).map_err(|_| BlobFacts::Facts(vec![]))
}

/// The directives of a go.mod or go.work, one-line and block form: (directive, its arguments).
fn go_directives(t: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut block: Option<String> = None;
    for line in t.lines() {
        let l = line.split("//").next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        if let Some(d) = &block {
            if l == ")" {
                block = None;
            } else {
                out.push((d.clone(), l.to_owned()));
            }
            continue;
        }
        let (d, rest) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
        let rest = rest.trim();
        if rest == "(" {
            block = Some(d.to_owned());
        } else {
            out.push((d.to_owned(), rest.to_owned()));
        }
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('`').to_owned()
}

impl Extractor for ManifestGo {
    fn name(&self) -> &'static str {
        "manifest-go"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[GO_MOD, GO_WORK]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        match base(path) {
            "go.mod" => Some(GO_MOD),
            "go.work" => Some(GO_WORK),
            _ => None,
        }
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        let t = match text(bytes) {
            Ok(t) => t,
            Err(f) => return f,
        };
        let d = go_directives(t);
        if kind == GO_WORK {
            let uses: Vec<String> = d
                .iter()
                .filter(|(k, _)| k == "use")
                .map(|(_, v)| unquote(v))
                .collect();
            return facts(vec![("use", json!(uses))]);
        }
        let module = d
            .iter()
            .find(|(k, _)| k == "module")
            .map(|(_, v)| unquote(v));
        // `replace old [v] => new [v]`: kept when the new side is a folder.
        let replace: Vec<[String; 2]> = d
            .iter()
            .filter(|(k, _)| k == "replace")
            .filter_map(|(_, v)| {
                let (old, new) = v.split_once("=>")?;
                let old = unquote(old.split_whitespace().next()?);
                let new = unquote(new.split_whitespace().next()?);
                (new.starts_with("./") || new.starts_with("../")).then_some([old, new])
            })
            .collect();
        facts(vec![
            ("module", json!(module)),
            (
                "replace",
                if replace.is_empty() {
                    Value::Null
                } else {
                    json!(replace)
                },
            ),
        ])
    }
}

impl Extractor for ManifestPy {
    fn name(&self) -> &'static str {
        "manifest-py"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[PYPROJECT, SETUP_CFG]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        match base(path) {
            "pyproject.toml" => Some(PYPROJECT),
            "setup.cfg" => Some(SETUP_CFG),
            _ => None,
        }
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        let t = match text(bytes) {
            Ok(t) => t,
            Err(f) => return f,
        };
        let (mut name, mut roots): (Option<String>, Vec<String>) = (None, Vec::new());
        if kind == PYPROJECT {
            let Ok(v) = t.parse::<toml::Table>() else {
                return BlobFacts::Facts(vec![]);
            };
            let get = |path: &[&str]| -> Option<&toml::Value> {
                let mut cur = v.get(path[0])?;
                for p in &path[1..] {
                    cur = cur.get(p)?;
                }
                Some(cur)
            };
            name = get(&["project", "name"])
                .or_else(|| get(&["tool", "poetry", "name"]))
                .and_then(|n| n.as_str())
                .map(str::to_owned);
            let strs = |x: Option<&toml::Value>| -> Vec<String> {
                x.and_then(|w| w.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            roots.extend(strs(get(&[
                "tool",
                "setuptools",
                "packages",
                "find",
                "where",
            ])));
            if let Some(d) =
                get(&["tool", "setuptools", "package-dir", ""]).and_then(|d| d.as_str())
            {
                roots.push(d.to_owned());
            }
            if let Some(pkgs) = get(&["tool", "poetry", "packages"]).and_then(|p| p.as_array()) {
                roots.extend(
                    pkgs.iter()
                        .filter_map(|p| p.get("from").and_then(|f| f.as_str()).map(str::to_owned)),
                );
            }
        } else {
            // setup.cfg: `name` under [metadata], `package_dir = =src` under [options].
            let mut section = String::new();
            for line in t.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    section = l.trim_matches(['[', ']']).to_owned();
                } else if let Some((k, v)) = l.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    if section == "metadata" && k == "name" {
                        name = Some(v.to_owned());
                    } else if section == "options" && k == "package_dir" {
                        let dir = v.trim_start_matches('=').trim();
                        if !dir.is_empty() {
                            roots.push(dir.to_owned());
                        }
                    }
                }
            }
        }
        roots.sort();
        roots.dedup();
        facts(vec![
            ("name", json!(name)),
            (
                "roots",
                if roots.is_empty() {
                    Value::Null
                } else {
                    json!(roots)
                },
            ),
        ])
    }
}

impl Extractor for ManifestPhp {
    fn name(&self) -> &'static str {
        "manifest-php"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[COMPOSER]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        (base(path) == "composer.json").then_some(COMPOSER)
    }
    fn extract(&self, _kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if let Some(r) = crate::skip_reason(bytes) {
            return BlobFacts::Skipped(r);
        }
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            return BlobFacts::Facts(vec![]);
        };
        // autoload and autoload-dev, merged: prefix -> folders.
        let map = |key: &str| -> Value {
            let mut out = serde_json::Map::new();
            for section in ["autoload", "autoload-dev"] {
                if let Some(o) = v
                    .pointer(&format!("/{section}/{key}"))
                    .and_then(Value::as_object)
                {
                    for (prefix, dirs) in o {
                        let list: Vec<Value> = match dirs {
                            Value::String(s) => vec![json!(s)],
                            Value::Array(a) => {
                                a.iter().filter(|x| x.is_string()).cloned().collect()
                            }
                            _ => vec![],
                        };
                        let e = out.entry(prefix.clone()).or_insert_with(|| json!([]));
                        if let Some(a) = e.as_array_mut() {
                            a.extend(list);
                        }
                    }
                }
            }
            if out.is_empty() {
                Value::Null
            } else {
                Value::Object(out)
            }
        };
        facts(vec![
            (
                "name",
                v.get("name")
                    .filter(|n| n.is_string())
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("psr-4", map("psr-4")),
            ("psr-0", map("psr-0")),
        ])
    }
}
