//! Protobuf (XS-18): a `.proto` file read, without `protoc`, into the same [`Contract`] an OpenAPI
//! spec gives. Each `rpc` of a `service` is an operation with method `RPC` and path
//! `/pkg.Service/Method` (the name gRPC puts on the wire); each `message` is a schema.
//!
//! A `.proto` file is hostile input like a spec: the grammar is tree-sitter's (error tolerant, a
//! time limit per parse), the walk is depth- and count-capped, and nothing is executed or fetched.
//! Imports are kept as written (`Contract::files`); the index resolves them against the repository
//! with [`resolve`], which never leaves the root it was given, so a path with `..` or a drive or
//! scheme in it is `outside` and a missing file is `missing`.

use tree_sitter::Node;

use crate::openapi::{
    self, Contract, MAX_OPERATIONS, MAX_SCHEMAS, MAX_SPEC_BYTES, Operation, Schema, SchemaField,
};
use crate::syntax::{parse, text};
use crate::{BlobFacts, Extractor, SkipReason};

/// The extractor's kind (its name is [`openapi::KIND`]'s sibling, `manifest-proto`).
pub const KIND: &str = "manifest-proto";
pub const METHOD: &str = "RPC";
const MAX_DEPTH: usize = 64;
const MAX_FIELDS: usize = 500;
/// Most imports kept per file, and most segments in one import path.
pub const MAX_IMPORTS: usize = 256;
pub const MAX_IMPORT_SEGMENTS: usize = 32;
/// Most folders above a file that are tried as import roots.
pub const MAX_ROOTS: usize = 32;

const SCALARS: [&str; 15] = [
    "double", "float", "int32", "int64", "uint32", "uint64", "sint32", "sint64", "fixed32",
    "fixed64", "sfixed32", "sfixed64", "bool", "string", "bytes",
];

/// `.proto` paths: the whole of what this extractor handles.
pub fn named(path: &str) -> bool {
    path.ends_with(".proto")
}

fn ty(n: Node, src: &[u8]) -> String {
    let t = text(n, src).trim_start_matches('.');
    if SCALARS.contains(&t) {
        t.to_owned()
    } else {
        format!("ref:{t}")
    }
}

fn first<'a>(n: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut c = n.walk();
    n.children(&mut c).find(|c| c.kind() == kind)
}

fn string(n: Node, src: &[u8]) -> Option<String> {
    let s = first(n, "string")
        .and_then(|s| first(s, "string_content"))
        .map_or("", |c| text(c, src));
    (!s.is_empty()).then(|| s.to_owned())
}

fn fields(body: Node, src: &[u8]) -> Vec<SchemaField> {
    let mut out = Vec::new();
    let mut c = body.walk();
    let mut kids = body.children(&mut c).collect::<Vec<_>>();
    // A oneof's members are fields of the message.
    let mut i = 0;
    while i < kids.len() && out.len() < MAX_FIELDS {
        let k = kids[i];
        i += 1;
        match k.kind() {
            "oneof" => {
                let mut oc = k.walk();
                kids.extend(k.children(&mut oc).filter(|c| c.kind() == "oneof_field"));
            }
            "field" | "oneof_field" | "map_field" => {
                let (Some(name), Some(t)) = (first(k, "identifier"), first(k, "type")) else {
                    continue;
                };
                let t = if k.kind() == "map_field" {
                    let key = first(k, "key_type").map_or("", |c| text(c, src));
                    format!("map<{key},{}>", ty(t, src))
                } else if text(k, src).starts_with("repeated") {
                    format!("array<{}>", ty(t, src))
                } else {
                    ty(t, src)
                };
                out.push(SchemaField {
                    name: text(name, src).to_owned(),
                    ty: t,
                    required: text(k, src).starts_with("required"),
                });
            }
            _ => {}
        }
    }
    out
}

struct Walk<'a> {
    src: &'a [u8],
    package: String,
    c: Contract,
}

impl Walk<'_> {
    fn message(&mut self, n: Node, prefix: &str, depth: usize) {
        let Some(name) = first(n, "message_name").map(|m| text(m, self.src)) else {
            return;
        };
        let name = format!("{prefix}{name}");
        let body = first(n, "message_body");
        if self.c.schemas.len() >= MAX_SCHEMAS {
            self.c.truncated = true;
            return;
        }
        self.c.schemas.push(Schema {
            name: name.clone(),
            fields: body.map(|b| fields(b, self.src)).unwrap_or_default(),
        });
        if let (Some(b), true) = (body, depth < MAX_DEPTH) {
            let mut cur = b.walk();
            for m in b.children(&mut cur).filter(|m| m.kind() == "message") {
                self.message(m, &format!("{name}."), depth + 1);
            }
        }
    }

    fn service(&mut self, n: Node) {
        let Some(name) = first(n, "service_name").map(|m| text(m, self.src)) else {
            return;
        };
        let mut cur = n.walk();
        for rpc in n.children(&mut cur).filter(|m| m.kind() == "rpc") {
            let Some(m) = first(rpc, "rpc_name").map(|m| text(m, self.src)) else {
                continue;
            };
            if self.c.operations.len() >= MAX_OPERATIONS {
                self.c.truncated = true;
                return;
            }
            let t = text(rpc, self.src);
            let (req, res) = t.split_once("returns").unwrap_or((t, ""));
            let (up, down) = (
                req.contains("stream"),
                res.split('{').next().unwrap_or("").contains("stream"),
            );
            let package = if self.package.is_empty() {
                String::new()
            } else {
                format!("{}.", self.package)
            };
            self.c.operations.push(Operation {
                method: METHOD.to_owned(),
                path: format!("/{package}{name}/{m}"),
                operation_id: Some(format!("{name}.{m}")),
                summary: Some(
                    match (up, down) {
                        (false, false) => "unary",
                        (true, false) => "client stream",
                        (false, true) => "server stream",
                        (true, true) => "bidirectional stream",
                    }
                    .to_owned(),
                ),
                line: rpc.start_position().row as u32 + 1,
            });
        }
    }
}

/// The contract of one `.proto` file; `None` when the time limit stopped the parse.
pub fn read(bytes: &[u8]) -> Option<Contract> {
    let tree = parse(KIND, |_| tree_sitter_proto::LANGUAGE.into(), bytes)?;
    let root = tree.root_node();
    let mut w = Walk {
        src: bytes,
        package: String::new(),
        c: Contract {
            version: "proto2".into(),
            bases: vec![String::new()],
            ..Default::default()
        },
    };
    let mut cur = root.walk();
    let tops: Vec<Node> = root.children(&mut cur).collect();
    // The package names every service, whichever comes first.
    for n in &tops {
        if n.kind() == "package"
            && let Some(p) = first(*n, "full_ident")
        {
            w.package = text(p, bytes).to_owned();
            w.c.title.clone_from(&w.package);
        }
    }
    for n in tops {
        match n.kind() {
            "syntax" => {
                if let Some(s) = string(n, bytes) {
                    w.c.version = s;
                }
            }
            "import" if w.c.files.len() < MAX_IMPORTS => {
                if let Some(p) = string(n, bytes) {
                    w.c.files.push(p);
                }
            }
            "message" => w.message(n, "", 0),
            "service" => w.service(n),
            _ => {}
        }
    }
    w.c.files.sort();
    w.c.files.dedup();
    Some(w.c)
}

/// The `manifest-proto` extractor: operations and schemas of a `.proto` blob as the facts of an
/// OpenAPI spec, so the contracts pass reads both the same way.
pub struct ManifestProto;

impl Extractor for ManifestProto {
    fn name(&self) -> &'static str {
        KIND
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[KIND]
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        named(path).then_some(KIND)
    }
    fn max_bytes(&self, _kind: &'static str) -> u64 {
        MAX_SPEC_BYTES
    }
    fn extract(&self, _kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if bytes[..bytes.len().min(8192)].contains(&0) {
            return BlobFacts::Skipped(SkipReason::Binary);
        }
        match read(bytes) {
            Some(c) => BlobFacts::Facts(openapi::facts(&c)),
            None => BlobFacts::Skipped(SkipReason::Timeout),
        }
    }
}

/// The folders above a file, deepest first, ending with `""` (the repository root): the roots an
/// import is tried against after those a `buf.yaml` names.
pub fn ancestors(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut d = dir.trim_matches('/');
    while !d.is_empty() && out.len() < MAX_ROOTS {
        out.push(d.to_owned());
        d = d.rsplit_once('/').map_or("", |x| x.0);
    }
    out.push(String::new());
    out
}

/// The import roots a `buf.yaml` (v1 or v2) or `buf.work.yaml` in folder `base` names: its
/// `modules[].path` and `directories`, else the folder itself. Paths that leave the repository
/// are dropped.
pub fn buf_roots(base: &str, yaml: &[u8]) -> Vec<String> {
    let Ok(v) = serde_norway::from_slice::<serde_norway::Value>(yaml) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let mut paths: Vec<&str> = Vec::new();
    if let Some(ms) = v.get("modules").and_then(|m| m.as_sequence()) {
        for m in ms.iter().take(MAX_ROOTS) {
            paths.push(m.get("path").and_then(|p| p.as_str()).unwrap_or("."));
        }
    }
    if let Some(ds) = v.get("directories").and_then(|m| m.as_sequence()) {
        paths.extend(ds.iter().take(MAX_ROOTS).filter_map(|d| d.as_str()));
    }
    out.extend(paths.iter().filter_map(|p| openapi::join(base, p)));
    if out.is_empty() {
        out.extend(openapi::join(base, "."));
    }
    out
}

/// Why an import was not resolved, or its repository-relative path. `roots` are tried in order;
/// `exists` says whether a repository-relative file is there. The well-known types protoc bundles
/// (`google/protobuf/*.proto`) are `bundled`, not missing.
pub fn resolve(
    import: &str,
    roots: &[String],
    exists: &mut dyn FnMut(&str) -> bool,
) -> Result<String, &'static str> {
    if import.starts_with("google/protobuf/") {
        return Err("bundled");
    }
    if import.split('/').count() > MAX_IMPORT_SEGMENTS {
        return Err("deep");
    }
    // Relative to nothing: a `..` that climbs, an absolute path or a drive or scheme is outside.
    let rel = openapi::join("", import).ok_or("outside")?;
    if rel.is_empty() {
        return Err("missing");
    }
    for root in roots {
        if let Some(p) = openapi::join(root, &rel)
            && exists(&p)
        {
            return Ok(p);
        }
    }
    Err("missing")
}

#[cfg(test)]
mod tests;
