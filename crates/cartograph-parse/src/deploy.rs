//! manifest-deploy (XS-02, C6, C7): the host names a compose file or a k8s manifest gives a
//! service, one fact each. They are what the index's host aliases are built from, so a blob's
//! aliases are extracted once and cached like any other fact.
//!
//! Compose gives a service its service name, `container_name` and `networks.*.aliases`; a k8s
//! `Service` gives `name`, `name.ns`, `name.ns.svc` and `name.ns.svc.cluster.local`. Only compose
//! services with a `build` are kept (an image-only service has no code in the repository), with
//! the `build` context as written: it is relative to the file, so the index resolves it against
//! the path. Read as data, never run; the YAML goes through [`yaml_docs`]'s limits.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

use crate::{BlobFacts, Extractor, Fact};

pub struct ManifestDeploy;

const COMPOSE: &str = "manifest-deploy/compose";
const K8S: &str = "manifest-deploy/k8s";

/// The most hosts one blob may give; a hostile file can't flood the facts.
const MAX_HOSTS: usize = 5_000;

/// The documents of a YAML file, as data. serde_norway's alias and depth limits bound a hostile
/// file (XS-30); at most 100 documents are read.
pub fn yaml_docs(text: &str) -> Vec<Value> {
    serde_norway::Deserializer::from_str(text)
        .take(100)
        .filter_map(|d| Value::deserialize(d).ok())
        .collect()
}

/// Whether `path` is in a folder named for k8s manifests, the only folders whose YAML is read as
/// such.
pub fn is_k8s_dir(path: &str) -> bool {
    path.rsplit_once('/').is_some_and(|(d, _)| {
        d.split('/')
            .any(|d| matches!(d, "k8s" | "kubernetes" | "kube" | "deploy" | "manifests"))
    })
}

/// Whether the file name is one of compose's.
pub fn is_compose_name(path: &str) -> bool {
    let b = path.rsplit('/').next().unwrap_or(path);
    (b.starts_with("docker-compose") || b.starts_with("compose"))
        && (b.ends_with(".yml") || b.ends_with(".yaml"))
}

/// A DNS-ish name worth keeping: lower-cased, 1 to 253 of letters, digits, `-`, `_` and `.`.
fn host_name(h: &str) -> Option<String> {
    let h = h.trim().trim_end_matches('.').to_ascii_lowercase();
    (!h.is_empty()
        && h.len() <= 253
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
    .then_some(h)
}

/// The container port a compose `ports` or `expose` entry names: `8080`, `"8080"`,
/// `"127.0.0.1:80:8080/tcp"` or `{target: 8080}`. A range or anything else names none.
fn container_port(v: &Value) -> Option<u16> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(s) => s.rsplit(':').next()?.split('/').next()?.trim().parse().ok(),
        Value::Object(o) => o.get("target").and_then(container_port),
        _ => None,
    }
}

/// The names a k8s Service answers to inside the cluster.
pub fn k8s_hosts(name: &str, namespace: &str) -> [String; 4] {
    [
        name.to_owned(),
        format!("{name}.{namespace}"),
        format!("{name}.{namespace}.svc"),
        format!("{name}.{namespace}.svc.cluster.local"),
    ]
}

/// The ports a list of entries names, or `[None]` (any port) when it names none.
fn ports_of<'a>(entries: impl Iterator<Item = &'a Value>) -> BTreeSet<Option<u16>> {
    let ports: BTreeSet<Option<u16>> = entries.filter_map(container_port).map(Some).collect();
    if ports.is_empty() {
        BTreeSet::from([None])
    } else {
        ports
    }
}

fn compose(doc: &Value, out: &mut Vec<Fact>) {
    let Some(services) = doc.get("services").and_then(Value::as_object) else {
        return;
    };
    for (name, def) in services {
        let context = match def.get("build") {
            Some(Value::String(s)) => Some(s.as_str()),
            Some(b) => b.get("context").and_then(Value::as_str),
            None => None,
        };
        let Some(context) = context else { continue };
        let mut hosts = vec![name.as_str()];
        if let Some(c) = def.get("container_name").and_then(Value::as_str) {
            hosts.push(c);
        }
        if let Some(nets) = def.get("networks").and_then(Value::as_object) {
            for net in nets.values() {
                if let Some(a) = net.get("aliases").and_then(Value::as_array) {
                    hosts.extend(a.iter().filter_map(Value::as_str));
                }
            }
        }
        let ports = ports_of(
            ["expose", "ports"]
                .iter()
                .filter_map(|k| def.get(k).and_then(Value::as_array))
                .flatten(),
        );
        for host in hosts.into_iter().filter_map(host_name) {
            for &port in &ports {
                out.push(Fact::Host {
                    host: host.clone(),
                    port,
                    context: Some(context.to_owned()),
                });
            }
        }
    }
}

fn k8s(doc: &Value, out: &mut Vec<Fact>) {
    if doc.get("kind").and_then(Value::as_str) != Some("Service") {
        return;
    }
    let Some(name) = doc.pointer("/metadata/name").and_then(Value::as_str) else {
        return;
    };
    let ns = doc
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let ports = ports_of(
        doc.pointer("/spec/ports")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("port")),
    );
    for host in k8s_hosts(name, ns)
        .into_iter()
        .filter_map(|h| host_name(&h))
    {
        for &port in &ports {
            out.push(Fact::Host {
                host: host.clone(),
                port,
                context: None,
            });
        }
    }
}

impl Extractor for ManifestDeploy {
    fn name(&self) -> &'static str {
        "manifest-deploy"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[COMPOSE, K8S]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        let yaml = path.ends_with(".yml") || path.ends_with(".yaml");
        if is_k8s_dir(path) {
            yaml.then_some(K8S)
        } else {
            is_compose_name(path).then_some(COMPOSE)
        }
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if let Some(r) = crate::skip_reason(bytes) {
            return BlobFacts::Skipped(r);
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return BlobFacts::Facts(vec![]);
        };
        let mut out = Vec::new();
        for doc in yaml_docs(text) {
            if kind == K8S {
                k8s(&doc, &mut out);
            } else {
                compose(&doc, &mut out);
            }
        }
        out.sort();
        out.dedup();
        out.truncate(MAX_HOSTS);
        BlobFacts::Facts(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(kind: &'static str, text: &str) -> Vec<String> {
        ManifestDeploy
            .extract(kind, text.as_bytes())
            .facts()
            .iter()
            .map(|f| match f {
                Fact::Host {
                    host,
                    port,
                    context,
                } => format!(
                    "{host}:{}@{}",
                    port.map_or("*".to_owned(), |p| p.to_string()),
                    context.as_deref().unwrap_or("")
                ),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn a_k8s_service_has_four_forms_in_its_namespace() {
        assert_eq!(
            k8s_hosts("orders", "shop"),
            [
                "orders",
                "orders.shop",
                "orders.shop.svc",
                "orders.shop.svc.cluster.local"
            ]
        );
        let y = "kind: Service\nmetadata: {name: Orders, namespace: shop}\nspec:\n  ports:\n    - port: 80\n";
        assert_eq!(
            hosts(K8S, y),
            [
                "orders:80@",
                "orders.shop:80@",
                "orders.shop.svc:80@",
                "orders.shop.svc.cluster.local:80@"
            ]
        );
        // No namespace is `default`; a Deployment is not a Service.
        let y =
            "kind: Service\nmetadata: {name: db}\n---\nkind: Deployment\nmetadata: {name: web}\n";
        assert_eq!(
            hosts(K8S, y),
            [
                "db:*@",
                "db.default:*@",
                "db.default.svc:*@",
                "db.default.svc.cluster.local:*@"
            ]
        );
    }

    #[test]
    fn compose_names_container_names_and_network_aliases_with_their_ports() {
        let y = "services:
  orders:
    build: ./orders
    container_name: orders-1
    expose: ['9000']
    ports: ['127.0.0.1:18080:8080/tcp', 8081, {target: 8082}]
    networks:
      back: {aliases: [Orders-API]}
      front:
  image-only:
    image: redis
  web:
    build: {context: ../web}
";
        let got = hosts(COMPOSE, y);
        let orders: Vec<&str> = got
            .iter()
            .filter(|h| h.ends_with("@./orders"))
            .map(String::as_str)
            .collect();
        assert_eq!(orders.len(), 3 * 4, "three names, four ports: {got:?}");
        assert!(orders.contains(&"orders-api:8080@./orders"), "{got:?}");
        assert!(orders.contains(&"orders-1:9000@./orders"), "{got:?}");
        assert!(orders.contains(&"orders:8082@./orders"), "{got:?}");
        assert!(got.contains(&"web:*@../web".to_owned()));
        assert_eq!(got.len(), 13, "no image-only service: {got:?}");
    }

    #[test]
    fn only_the_fixed_names_are_handled() {
        for (p, kind) in [
            ("docker-compose.yml", COMPOSE),
            ("ops/compose.prod.yaml", COMPOSE),
            ("docker-compose.override.yml", COMPOSE),
            ("k8s/svc.yaml", K8S),
            ("infra/deploy/a/b.yml", K8S),
        ] {
            assert_eq!(ManifestDeploy.handles(p), Some(kind), "{p}");
        }
        for p in [
            ".github/workflows/ci.yml",
            "config.yaml",
            "k8s/readme.md",
            "deploy.yml",
            "compose.json",
        ] {
            assert_eq!(ManifestDeploy.handles(p), None, "{p}");
        }
    }

    #[test]
    fn hostile_yaml_is_bounded() {
        let mut y = String::from("a: &a [x, x]\n");
        for i in 0..40 {
            let p = if i == 0 {
                "a".to_owned()
            } else {
                format!("l{}", i - 1)
            };
            y.push_str(&format!("l{i}: &l{i} [*{p}, *{p}]\n"));
        }
        let _ = hosts(COMPOSE, &y);
        let deep = format!("services: {}", "[".repeat(10_000));
        assert!(hosts(COMPOSE, &deep).is_empty());
        assert!(matches!(
            ManifestDeploy.extract(COMPOSE, &[0u8; 100]),
            BlobFacts::Skipped(_)
        ));
    }
}
