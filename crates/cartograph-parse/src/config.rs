//! Config value sources (XS-09): the keys and values a `.env.example`, a compose `environment:`, a
//! k8s ConfigMap or container `env`, a Helm values file and the `{{ .Values.a.b }}` lookups of a
//! Helm template give, one fact each. Read as data and never run; Helm templates are not rendered.
//!
//! A value is kept only when it classifies as a locator (XS-10); anything else is a key with no
//! value. A k8s `Secret` and a `secretKeyRef` give key names only. Real `.env` files are not read,
//! only `*.example`, `*.sample` and `*.template`.

use cartograph_core::secrets::{ConfigValue, classify_config_value};
use serde_json::Value;

use crate::{ConfigScope, ConfigVal, Fact};

/// Most facts one blob may give.
pub const MAX_CONFIG: usize = 5_000;
const MAX_DEPTH: usize = 8;
/// How a `{{ .Values.a.b }}` lookup is carried through the YAML parse.
const MARK: &str = "__values__";

fn env_fact(key: &str, value: Option<&str>) -> Fact {
    let value = match value {
        Some(v) => match classify_config_value(key, v) {
            ConfigValue::Locator(l) => ConfigVal::Locator(l),
            ConfigValue::Opaque => ConfigVal::Opaque,
        },
        None => ConfigVal::Opaque,
    };
    Fact::Config {
        scope: ConfigScope::Env,
        key: key.to_owned(),
        value,
    }
}

/// An env var name worth keeping.
fn valid_key(k: &str) -> bool {
    (1..=128).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A file name that is one of the example forms of `.env`: `.env.example`, `.env.prod.example`,
/// `.env.sample`, `.env.template`.
pub fn is_dotenv_example(path: &str) -> bool {
    let b = path.rsplit('/').next().unwrap_or(path);
    b.starts_with(".env")
        && [".example", ".sample", ".template"]
            .iter()
            .any(|s| b.ends_with(s))
}

/// `values.yaml`, `values-prod.yaml`, `values.prod.yml`, `values_prod.yaml`.
pub fn is_values_name(path: &str) -> bool {
    let b = path.rsplit('/').next().unwrap_or(path);
    let Some(stem) = b.strip_suffix(".yaml").or_else(|| b.strip_suffix(".yml")) else {
        return false;
    };
    stem == "values"
        || stem
            .strip_prefix("values")
            .is_some_and(|r| r.starts_with(['-', '.', '_']) && r.len() > 1)
}

/// Whether a YAML file sits in a Helm chart's `templates` folder.
pub fn is_helm_template(path: &str) -> bool {
    (path.ends_with(".yaml") || path.ends_with(".yml"))
        && path
            .rsplit_once('/')
            .is_some_and(|(d, _)| d.split('/').any(|s| s == "templates"))
}

/// `KEY=value` lines.
pub fn dotenv(text: &str, out: &mut Vec<Fact>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if !valid_key(k) {
            continue;
        }
        let v = v.trim();
        let v = match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
            // An unquoted value ends at ` #`.
            _ => v.split(" #").next().unwrap_or("").trim(),
        };
        out.push(env_fact(k, (!v.is_empty()).then_some(v)));
    }
}

/// A compose service's `environment:`, a map or a list of `KEY=value`.
pub fn compose_env(def: &Value, out: &mut Vec<Fact>) {
    match def.get("environment") {
        Some(Value::Object(m)) => {
            for (k, v) in m {
                if valid_key(k) {
                    out.push(env_fact(k, scalar(v).as_deref()));
                }
            }
        }
        Some(Value::Array(a)) => {
            for e in a.iter().filter_map(Value::as_str) {
                let (k, v) = e.split_once('=').map_or((e, None), |(k, v)| (k, Some(v)));
                if valid_key(k) {
                    out.push(env_fact(k, v.filter(|v| !v.is_empty())));
                }
            }
        }
        _ => {}
    }
}

/// A k8s document: ConfigMap `data`, container `env` anywhere in a workload, and the key names of
/// a Secret.
pub fn k8s_env(doc: &Value, out: &mut Vec<Fact>) {
    match doc.get("kind").and_then(Value::as_str) {
        Some("ConfigMap") => {
            for k in ["data"] {
                for (name, v) in doc.get(k).and_then(Value::as_object).into_iter().flatten() {
                    if valid_key(name) {
                        out.push(env_fact(name, scalar(v).as_deref()));
                    }
                }
            }
        }
        Some("Secret") => {
            for k in ["data", "stringData"] {
                for name in doc
                    .get(k)
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|m| m.keys())
                {
                    if valid_key(name) {
                        out.push(env_fact(name, None));
                    }
                }
            }
        }
        _ => {
            let mut found = Vec::new();
            container_env_raw(doc, 0, &mut found);
            // `valueFrom` (a Secret or ConfigMap key) gives the name only.
            out.extend(found.iter().map(|(n, v)| env_fact(n, v.as_deref())));
        }
    }
}

fn flatten(prefix: &str, v: &Value, depth: usize, out: &mut Vec<Fact>) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, x, depth + 1, out);
            }
        }
        Value::String(s) => {
            let value = match classify_config_value(prefix, s) {
                ConfigValue::Locator(l) => ConfigVal::Locator(l),
                ConfigValue::Opaque => ConfigVal::Opaque,
            };
            out.push(Fact::Config {
                scope: ConfigScope::Values,
                key: prefix.to_owned(),
                value,
            });
        }
        _ => {}
    }
}

/// Every string leaf of a Helm values document under its dotted path.
pub fn helm_values(doc: &Value, out: &mut Vec<Fact>) {
    flatten("", doc, 0, out);
}

/// The dotted path of a direct `{{ .Values.a.b }}` lookup (a `| quote` is allowed); `None` for
/// anything else a template does.
fn values_ref(expr: &str) -> Option<&str> {
    let e = expr.trim().trim_matches('-').trim();
    let e = e
        .strip_suffix("| quote")
        .or_else(|| e.strip_suffix("| squote"))
        .unwrap_or(e)
        .trim();
    let path = e.strip_prefix(".Values.")?;
    (!path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        && !path.ends_with('.'))
    .then_some(path)
}

/// A template made parseable: direct `.Values` lookups become `__values__a.b` and any other
/// action is dropped (a whole line of control flow) or becomes an empty string.
pub fn helm_strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if !line.contains("{{") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.trim_start().starts_with("{{") {
            out.push('\n');
            continue;
        }
        let mut rest = line;
        while let Some(i) = rest.find("{{") {
            out.push_str(&rest[..i]);
            let Some(j) = rest[i..].find("}}") else {
                rest = "";
                break;
            };
            let expr = &rest[i + 2..i + j];
            match values_ref(expr) {
                Some(p) => {
                    out.push_str(MARK);
                    out.push_str(p);
                }
                None => out.push_str("''"),
            }
            rest = &rest[i + j + 2..];
        }
        out.push_str(rest);
        out.push('\n');
    }
    out
}

/// The container `env` of a stripped Helm template: a value that is a direct lookup is a `Ref`.
pub fn helm_template_env(doc: &Value, out: &mut Vec<Fact>) {
    let mut found = Vec::new();
    container_env_raw(doc, 0, &mut found);
    for (name, value) in found {
        let fact = match value.as_deref().and_then(|v| v.strip_prefix(MARK)) {
            Some(p) => Fact::Config {
                scope: ConfigScope::Env,
                key: name,
                value: ConfigVal::Ref(p.to_owned()),
            },
            None => env_fact(&name, value.as_deref()),
        };
        out.push(fact);
    }
}

fn container_env_raw(v: &Value, depth: usize, out: &mut Vec<(String, Option<String>)>) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if matches!(k.as_str(), "containers" | "initContainers") {
                    for c in x.as_array().into_iter().flatten() {
                        for e in c.get("env").and_then(Value::as_array).into_iter().flatten() {
                            if let Some(n) = e.get("name").and_then(Value::as_str)
                                && valid_key(n)
                            {
                                out.push((n.to_owned(), e.get("value").and_then(scalar)));
                            }
                        }
                    }
                } else {
                    container_env_raw(x, depth + 1, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| container_env_raw(x, depth + 1, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::yaml_docs;

    fn show(f: &[Fact]) -> Vec<String> {
        f.iter()
            .map(|f| match f {
                Fact::Config { scope, key, value } => format!("{scope:?} {key} {value:?}"),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn dotenv_keeps_locators_and_drops_everything_else() {
        let mut out = Vec::new();
        dotenv(
            "# c\nCATALOG_URL=http://catalog.local:8080\nexport PG=postgres://u:pw@db/app\nAWS_KEY=\"AKIAIOSFODNN7EXAMPLE\"\nEMPTY=\nQ='quoted:1' # x\nnot a pair\n",
            &mut out,
        );
        let s = show(&out).join("\n");
        assert!(
            s.contains("Env CATALOG_URL Locator(\"http://catalog.local:8080\")"),
            "{s}"
        );
        // Userinfo is dropped from a URL, a deny-listed key keeps no value.
        assert!(s.contains("Env PG Locator(\"postgres://db/app\")"), "{s}");
        assert!(!s.contains("pw"), "{s}");
        assert!(s.contains("Env AWS_KEY Opaque"), "{s}");
        assert!(!s.contains("AKIA"), "{s}");
        assert!(s.contains("Env EMPTY Opaque"), "{s}");
        assert!(s.contains("Env Q Locator(\"quoted:1\")"), "{s}");
    }

    #[test]
    fn only_example_forms_are_dotenv() {
        for p in [".env.example", "a/.env.prod.example", ".env.sample"] {
            assert!(is_dotenv_example(p), "{p}");
        }
        for p in [".env", ".env.local", "env.example", ".envrc"] {
            assert!(!is_dotenv_example(p), "{p}");
        }
    }

    #[test]
    fn compose_environment_as_map_and_list() {
        let docs = yaml_docs(
            "services:\n  web:\n    environment:\n      CATALOG_URL: http://catalog:8080\n      N: 3\n  api:\n    environment:\n      - PRICING=http://pricing:9000\n      - BARE\n",
        );
        let mut out = Vec::new();
        for (_, def) in docs[0]["services"].as_object().unwrap() {
            compose_env(def, &mut out);
        }
        let s = show(&out).join("\n");
        assert!(
            s.contains("CATALOG_URL Locator(\"http://catalog:8080\")"),
            "{s}"
        );
        assert!(
            s.contains("PRICING Locator(\"http://pricing:9000\")"),
            "{s}"
        );
        assert!(s.contains("Env BARE Opaque"), "{s}");
    }

    #[test]
    fn k8s_secrets_and_secret_refs_give_names_only() {
        let docs = yaml_docs(
            "kind: Secret\nmetadata: {name: s}\nstringData:\n  DB_URL: postgres://u:hunter2@db/x\n---\nkind: ConfigMap\ndata:\n  CATALOG_URL: http://catalog.prod.svc:8080\n---\nkind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: a\n          env:\n            - {name: PRICING_URL, value: 'http://pricing.prod.svc'}\n            - name: TOKEN\n              valueFrom: {secretKeyRef: {name: s, key: t}}\n",
        );
        let mut out = Vec::new();
        for d in &docs {
            k8s_env(d, &mut out);
        }
        let s = show(&out).join("\n");
        assert!(s.contains("Env DB_URL Opaque"), "{s}");
        assert!(!s.contains("hunter2"), "{s}");
        assert!(
            s.contains("CATALOG_URL Locator(\"http://catalog.prod.svc:8080\")"),
            "{s}"
        );
        assert!(s.contains("PRICING_URL Locator"), "{s}");
        assert!(s.contains("Env TOKEN Opaque"), "{s}");
    }

    #[test]
    fn helm_values_flatten_and_templates_substitute_direct_lookups_only() {
        let docs = yaml_docs(
            "catalog:\n  url: http://catalog.prod.svc:8080\n  replicas: 2\nauth:\n  password: hunter2\n",
        );
        let mut out = Vec::new();
        helm_values(&docs[0], &mut out);
        let s = show(&out).join("\n");
        assert!(
            s.contains("Values catalog.url Locator(\"http://catalog.prod.svc:8080\")"),
            "{s}"
        );
        assert!(s.contains("Values auth.password Opaque"), "{s}");
        assert!(!s.contains("hunter2"), "{s}");

        let t = "{{- if .Values.on }}\nkind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n          env:\n            - name: CATALOG_URL\n              value: {{ .Values.catalog.url | quote }}\n            - name: OTHER\n              value: {{ include \"x\" . }}\n            - name: LIT\n              value: \"http://lit:1\"\n{{- end }}\n";
        let docs = yaml_docs(&helm_strip(t));
        let mut out = Vec::new();
        helm_template_env(&docs[0], &mut out);
        let s = show(&out).join("\n");
        assert!(s.contains("Env CATALOG_URL Ref(\"catalog.url\")"), "{s}");
        assert!(s.contains("Env OTHER Opaque"), "{s}");
        assert!(s.contains("Env LIT Locator(\"http://lit:1\")"), "{s}");
    }

    #[test]
    fn values_file_names() {
        for p in [
            "values.yaml",
            "c/values-prod.yaml",
            "values.prod.yml",
            "values_x.yaml",
        ] {
            assert!(is_values_name(p), "{p}");
        }
        for p in [
            "valuesx.yaml",
            "values.json",
            "my-values.yaml",
            "values-.yaml",
        ] {
            assert!(!is_values_name(p), "{p}");
        }
    }
}
