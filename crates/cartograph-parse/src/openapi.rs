//! OpenAPI 3.0, 3.1 and Swagger 2.0 (XS-08) read into one model: a [`Contract`] of operations
//! (method, path, `operationId`) and schemas with their fields.
//!
//! A spec is hostile input. The parser reads data and never runs anything: it caps the size, the
//! nesting depth, the operations and the schemas, stops at a time budget, follows only `$ref`s
//! that stay in the document or name a file below the root file's folder (read through
//! [`Files`], which the index backs with git trees), and never fetches a remote one. Descriptions
//! and examples are not kept; an operation's summary is cut to [`MAX_SUMMARY`] characters.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BlobFacts, Extractor, Fact, SkipReason};

/// Spec files may be this big; the general parse cap is `MAX_BLOB_BYTES`.
pub const MAX_SPEC_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_OPERATIONS: usize = 20_000;
pub const MAX_SCHEMAS: usize = 50_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_SUMMARY: usize = 200;
const MAX_FIELDS: usize = 500;
const MAX_BASES: usize = 8;
const MAX_FILES: usize = 256;
const MAX_UNRESOLVED: usize = 1_000;
const BUDGET: Duration = Duration::from_secs(5);
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// One property of a schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SchemaField {
    pub name: String,
    /// `string`, `integer|null`, `array<ref:Item>`, `ref:Order`.
    pub ty: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<SchemaField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    /// Lower case: `get`.
    pub method: String,
    /// As the spec writes it, without a base: `/orders/{id}`.
    pub path: String,
    pub operation_id: Option<String>,
    /// At most [`MAX_SUMMARY`] characters. Untrusted text.
    pub summary: Option<String>,
    /// The line of the path in the root file; 1 when it isn't found.
    pub line: u32,
}

/// A `$ref` that was not followed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Unresolved {
    pub target: String,
    /// `remote`, `missing`, `outside` (leaves the repository), `cycle` or `deep`.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Contract {
    /// `2.0`, `3.0.3`, `3.1.0`.
    pub version: String,
    pub title: String,
    /// Path prefixes from `servers[].url` or `basePath`; `[""]` when there is none.
    pub bases: Vec<String>,
    pub operations: Vec<Operation>,
    pub schemas: Vec<Schema>,
    /// Files the spec reads through relative `$ref`s: repository-relative when parsed with a
    /// [`Files`], as written when not.
    pub files: Vec<String>,
    pub unresolved: Vec<Unresolved>,
    /// A cap cut the operations or the schemas short.
    pub truncated: bool,
}

/// Why a document was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Nested past [`MAX_DEPTH`], or past the time budget.
    Hostile,
}

/// The files a spec's relative `$ref`s name, as the repository has them at one commit.
pub trait Files {
    /// The bytes of a repository-relative path, `None` when it isn't there or is too big.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
}

/// Where a spec lives and what it may read.
pub struct Env<'a> {
    /// The root file's folder, repository-relative, no trailing slash.
    pub dir: &'a str,
    pub files: &'a mut dyn Files,
}

/// Whether a file name says it is a spec: `openapi.yaml`, `swagger.json`, `petstore-openapi.yml`.
pub fn named(path: &str) -> bool {
    let b = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    (b.ends_with(".yaml") || b.ends_with(".yml") || b.ends_with(".json"))
        && (b.contains("openapi") || b.contains("swagger"))
}

/// Whether the file is YAML or JSON at all.
pub fn is_data(path: &str) -> bool {
    path.ends_with(".yaml") || path.ends_with(".yml") || path.ends_with(".json")
}

/// `dir/rest` normalised; `None` when it climbs out of the repository or is absolute.
pub fn join(dir: &str, rest: &str) -> Option<String> {
    if rest.starts_with('/') || rest.starts_with('\\') || rest.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
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

fn cut(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
}

/// Nesting deeper than the cap, found without recursion.
fn too_deep(v: &Value) -> bool {
    let mut stack = vec![(v, 1usize)];
    while let Some((v, d)) = stack.pop() {
        if d > MAX_DEPTH {
            return true;
        }
        match v {
            Value::Object(o) => stack.extend(o.values().map(|x| (x, d + 1))),
            Value::Array(a) => stack.extend(a.iter().map(|x| (x, d + 1))),
            _ => {}
        }
    }
    false
}

/// JSON or YAML text as data.
fn load(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim_start().starts_with('{') {
        serde_json::from_str(text).ok()
    } else {
        serde_norway::from_str(text).ok()
    }
}

fn unescape(seg: &str) -> String {
    let mut out = Vec::with_capacity(seg.len());
    let b = seg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(h) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out)
        .replace("~1", "/")
        .replace("~0", "~")
}

/// The value a JSON pointer (`/components/schemas/Order`, or empty for the document) names.
fn pointer<'a>(doc: &'a Value, ptr: &str) -> Option<&'a Value> {
    let mut cur = doc;
    for seg in ptr.strip_prefix('/').into_iter().flat_map(|p| p.split('/')) {
        let seg = unescape(seg);
        cur = match cur {
            Value::Object(o) => o.get(&seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// `/v1` of `https://{host}/v1/`, with a server's variable defaults filled in.
fn base_of(server: &Value) -> Option<String> {
    let mut url = server.get("url")?.as_str()?.to_owned();
    if let Some(vars) = server.get("variables").and_then(Value::as_object) {
        for (k, v) in vars {
            if let Some(d) = v.get("default").and_then(Value::as_str) {
                url = url.replace(&format!("{{{k}}}"), d);
            }
        }
    }
    Some(base_path(&url))
}

fn base_path(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let rest = match url.split_once("://") {
        Some((_, r)) => r.find('/').map_or("", |i| &r[i..]),
        None => url,
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.starts_with('/') {
        rest.to_owned()
    } else {
        format!("/{rest}")
    }
}

/// The line of each path key (`  /orders/{id}:` or `"/orders/{id}":`) of the root file.
fn path_lines(text: &str) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        let t = t.strip_prefix(['"', '\'']).unwrap_or(t);
        if t.starts_with('/')
            && let Some(end) = t.find(['"', '\'', ':'])
        {
            out.entry(t[..end].to_owned()).or_insert(i as u32 + 1);
        }
    }
    out
}

/// A document of the spec: the root or a file a `$ref` reached.
struct Doc {
    dir: String,
    value: Value,
}

struct Parser<'a> {
    docs: Vec<Doc>,
    index: HashMap<String, usize>,
    env: Option<Env<'a>>,
    started: Instant,
    files: Vec<String>,
    unresolved: BTreeMap<(String, &'static str), ()>,
}

impl Parser<'_> {
    fn over_budget(&self) -> bool {
        self.started.elapsed() > BUDGET
    }

    fn fail(&mut self, target: &str, reason: &'static str) {
        if self.unresolved.len() < MAX_UNRESOLVED {
            self.unresolved.insert((cut(target, 512), reason), ());
        }
    }

    fn load_file(&mut self, path: &str) -> Result<usize, &'static str> {
        if let Some(i) = self.index.get(path) {
            return Ok(*i);
        }
        if self.docs.len() > MAX_FILES {
            return Err("deep");
        }
        let env = self.env.as_mut().ok_or("missing")?;
        let bytes = env.files.read(path).ok_or("missing")?;
        let value = load(&bytes).ok_or("missing")?;
        if too_deep(&value) {
            return Err("deep");
        }
        self.files.push(path.to_owned());
        let dir = path.rsplit_once('/').map_or("", |x| x.0).to_owned();
        self.docs.push(Doc { dir, value });
        self.index.insert(path.to_owned(), self.docs.len() - 1);
        Ok(self.docs.len() - 1)
    }

    /// The value a `$ref` chain ends at, and the document it lives in. Without a file reader a
    /// relative file is only recorded.
    fn deref(&mut self, v: &Value, doc: usize) -> Option<(Value, usize)> {
        let mut cur = v.clone();
        let mut at = doc;
        let mut seen: HashSet<(usize, String)> = HashSet::new();
        for _ in 0..MAX_DEPTH {
            let Some(target) = cur.get("$ref").and_then(Value::as_str).map(str::to_owned) else {
                return Some((cur, at));
            };
            if !seen.insert((at, target.clone())) {
                self.fail(&target, "cycle");
                return None;
            }
            let (file, ptr) = target.split_once('#').unwrap_or((target.as_str(), ""));
            let next = if file.is_empty() {
                (at, pointer(&self.docs[at].value, ptr).cloned())
            } else if file.contains("://") {
                self.fail(&target, "remote");
                return None;
            } else if self.env.is_none() {
                let f = cut(file, 512);
                if !self.files.contains(&f) && self.files.len() < MAX_FILES {
                    self.files.push(f);
                }
                return None;
            } else {
                let Some(path) = join(&self.docs[at].dir, file) else {
                    self.fail(&target, "outside");
                    return None;
                };
                match self.load_file(&path) {
                    Ok(i) => (i, pointer(&self.docs[i].value, ptr).cloned()),
                    Err(why) => {
                        self.fail(&target, why);
                        return None;
                    }
                }
            };
            let (i, Some(value)) = next else {
                self.fail(&target, "missing");
                return None;
            };
            at = i;
            cur = value;
        }
        self.fail("$ref chain", "deep");
        None
    }

    fn ty(&mut self, s: &Value) -> String {
        if let Some(r) = s.get("$ref").and_then(Value::as_str) {
            return format!("ref:{}", r.rsplit(['/', '#']).next().unwrap_or(r));
        }
        let base = match s.get("type") {
            Some(Value::String(t)) => t.clone(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("|"),
            _ if s.get("properties").is_some() => "object".into(),
            _ => "any".into(),
        };
        let out = match (base.as_str(), s.get("items")) {
            ("array", Some(i)) if i.is_object() => {
                let inner = match i.get("$ref").and_then(Value::as_str) {
                    Some(r) => format!("ref:{}", r.rsplit(['/', '#']).next().unwrap_or(r)),
                    None => match i.get("type").and_then(Value::as_str) {
                        Some(t) => t.to_owned(),
                        None => "any".into(),
                    },
                };
                format!("array<{inner}>")
            }
            _ => base,
        };
        cut(&out, 120)
    }

    fn schema(&mut self, name: &str, v: &Value, doc: usize) -> Option<Schema> {
        let (s, at) = self.deref(v, doc)?;
        let mut fields: BTreeMap<String, SchemaField> = BTreeMap::new();
        let mut parts = vec![s.clone()];
        if let Some(all) = s.get("allOf").and_then(Value::as_array) {
            for p in all.iter().take(16) {
                if let Some((p, _)) = self.deref(p, at) {
                    parts.push(p);
                }
            }
        }
        for part in &parts {
            let required: HashSet<&str> = part
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            for (n, p) in part
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .take(MAX_FIELDS)
            {
                let ty = self.ty(p);
                fields.insert(
                    cut(n, 200),
                    SchemaField {
                        name: cut(n, 200),
                        ty,
                        required: required.contains(n.as_str()),
                    },
                );
            }
        }
        Some(Schema {
            name: cut(name, 200),
            fields: fields.into_values().take(MAX_FIELDS).collect(),
        })
    }
}

/// Reads one document. `None` when it is not an OpenAPI or Swagger 2.0 document; `Err` when it
/// is hostile (nested past [`MAX_DEPTH`], or past the time budget). With `env` the relative-file
/// `$ref`s are followed and the files they read listed in [`Contract::files`], repository-relative;
/// without it they are listed as written and nothing is read.
pub fn parse(bytes: &[u8], env: Option<Env<'_>>) -> Result<Option<Contract>, Refused> {
    let started = Instant::now();
    if bytes.len() as u64 > MAX_SPEC_BYTES {
        return Ok(None);
    }
    // A cheap look first: most YAML and JSON files are not specs.
    let has = |n: &[u8]| bytes.windows(n.len()).any(|w| w == n);
    if !has(b"openapi") && !has(b"swagger") {
        return Ok(None);
    }
    let Some(root) = load(bytes) else {
        return Ok(None);
    };
    let version = match (root.get("openapi"), root.get("swagger")) {
        (Some(Value::String(v)), _) if v.starts_with("3.") => v.clone(),
        (_, Some(Value::String(v))) if v == "2.0" => v.clone(),
        (_, Some(Value::Number(n))) if n.as_f64() == Some(2.0) => "2.0".to_owned(),
        _ => return Ok(None),
    };
    if too_deep(&root) {
        return Err(Refused::Hostile);
    }
    let dir = env.as_ref().map_or("", |e| e.dir).to_owned();
    let lines = std::str::from_utf8(bytes)
        .map(path_lines)
        .unwrap_or_default();
    let mut p = Parser {
        docs: vec![Doc { dir, value: root }],
        index: HashMap::new(),
        env,
        started,
        files: Vec::new(),
        unresolved: BTreeMap::new(),
    };
    let root = p.docs[0].value.clone();
    let mut c = Contract {
        version: cut(&version, 32),
        title: cut(
            root.pointer("/info/title")
                .and_then(Value::as_str)
                .unwrap_or(""),
            200,
        ),
        ..Default::default()
    };
    // Base paths.
    let mut bases: Vec<String> = if version == "2.0" {
        vec![
            root.get("basePath")
                .and_then(Value::as_str)
                .map(base_path)
                .unwrap_or_default(),
        ]
    } else {
        root.get("servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(base_of)
            .collect()
    };
    bases.sort();
    bases.dedup();
    bases.truncate(MAX_BASES);
    if bases.is_empty() {
        bases.push(String::new());
    }
    c.bases = bases;

    // Operations: path item, then each method.
    let mut paths: Vec<(&String, &Value)> = root
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.starts_with('/'))
        .collect();
    paths.sort_by(|a, b| a.0.cmp(b.0));
    'paths: for (path, item) in paths {
        if p.over_budget() {
            return Err(Refused::Hostile);
        }
        let Some((item, _)) = p.deref(item, 0) else {
            continue;
        };
        for m in METHODS {
            let Some(op) = item.get(m).filter(|o| o.is_object()) else {
                continue;
            };
            if c.operations.len() >= MAX_OPERATIONS {
                c.truncated = true;
                break 'paths;
            }
            c.operations.push(Operation {
                method: m.to_owned(),
                path: cut(path, 2048),
                operation_id: op
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map(|s| cut(s, 256)),
                summary: op
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(|s| cut(s, MAX_SUMMARY)),
                line: lines.get(path.as_str()).copied().unwrap_or(1),
            });
        }
    }

    // Schemas.
    let defs = if version == "2.0" {
        root.get("definitions")
    } else {
        root.pointer("/components/schemas")
    };
    let mut named: Vec<(&String, &Value)> = defs
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .collect();
    named.sort_by(|a, b| a.0.cmp(b.0));
    for (name, s) in named {
        if c.schemas.len() >= MAX_SCHEMAS {
            c.truncated = true;
            break;
        }
        if p.over_budget() {
            return Err(Refused::Hostile);
        }
        if let Some(s) = p.schema(name, s, 0) {
            c.schemas.push(s);
        }
    }

    p.files.sort();
    p.files.dedup();
    c.files = p.files;
    c.unresolved = p
        .unresolved
        .into_keys()
        .map(|(target, reason)| Unresolved {
            target,
            reason: reason.to_owned(),
        })
        .collect();
    Ok(Some(c))
}

/// The `manifest-openapi` extractor: a spec's operations, schemas and unresolved refs as facts,
/// from one blob alone. A spec that reads other files through relative refs says which; the index
/// reads it again with those files (see `contracts` in the index crate).
pub struct ManifestOpenapi;

/// The kind [`ManifestOpenapi`] reads.
pub const KIND: &str = "manifest-openapi";

impl Extractor for ManifestOpenapi {
    fn name(&self) -> &'static str {
        KIND
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &[KIND]
    }
    /// Any YAML or JSON path: which of them the walk keeps is the index's call (names and the
    /// manifest's `contracts:` globs), so a file that is read here is one that was asked for.
    fn handles(&self, path: &str) -> Option<&'static str> {
        is_data(path).then_some(KIND)
    }
    fn max_bytes(&self, _kind: &'static str) -> u64 {
        MAX_SPEC_BYTES
    }
    fn extract(&self, _kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if bytes[..bytes.len().min(8192)].contains(&0) {
            return BlobFacts::Skipped(SkipReason::Binary);
        }
        let c = match parse(bytes, None) {
            Ok(Some(c)) => c,
            Ok(None) => return BlobFacts::Facts(vec![]),
            Err(Refused::Hostile) => return BlobFacts::Skipped(SkipReason::Timeout),
        };
        BlobFacts::Facts(facts(&c))
    }
}

/// A contract as sorted facts.
pub fn facts(c: &Contract) -> Vec<Fact> {
    let mut out = vec![Fact::Spec {
        version: c.version.clone(),
        title: c.title.clone(),
        bases: c.bases.clone(),
        files: c.files.clone(),
        truncated: c.truncated,
    }];
    out.extend(c.operations.iter().map(|o| Fact::Operation {
        method: o.method.clone(),
        path: o.path.clone(),
        operation_id: o.operation_id.clone(),
        summary: o.summary.clone(),
        line: o.line,
    }));
    out.extend(c.schemas.iter().map(|s| Fact::Schema {
        name: s.name.clone(),
        fields: s.fields.clone(),
    }));
    out.extend(c.unresolved.iter().map(|u| Fact::SpecRef {
        target: u.target.clone(),
        reason: u.reason.clone(),
    }));
    out.sort();
    out.dedup();
    out
}

/// The contract a blob's facts say, or `None` when the blob is not a spec.
pub fn from_facts(facts: &[Fact]) -> Option<Contract> {
    let mut c = Contract::default();
    let mut seen = false;
    for f in facts {
        match f {
            Fact::Spec {
                version,
                title,
                bases,
                files,
                truncated,
            } => {
                seen = true;
                c.version.clone_from(version);
                c.title.clone_from(title);
                c.bases.clone_from(bases);
                c.files.clone_from(files);
                c.truncated = *truncated;
            }
            Fact::Operation {
                method,
                path,
                operation_id,
                summary,
                line,
            } => c.operations.push(Operation {
                method: method.clone(),
                path: path.clone(),
                operation_id: operation_id.clone(),
                summary: summary.clone(),
                line: *line,
            }),
            Fact::Schema { name, fields } => c.schemas.push(Schema {
                name: name.clone(),
                fields: fields.clone(),
            }),
            Fact::SpecRef { target, reason } => c.unresolved.push(Unresolved {
                target: target.clone(),
                reason: reason.clone(),
            }),
            _ => {}
        }
    }
    seen.then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct None_;
    impl Files for None_ {
        fn read(&mut self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }

    struct Map(HashMap<String, String>);
    impl Files for Map {
        fn read(&mut self, p: &str) -> Option<Vec<u8>> {
            self.0.get(p).map(|s| s.clone().into_bytes())
        }
    }

    fn ops(c: &Contract) -> Vec<String> {
        c.operations
            .iter()
            .map(|o| format!("{} {}", o.method, o.path))
            .collect()
    }

    const V3: &str = r#"
openapi: 3.0.3
info: {title: Orders}
servers:
  - url: https://api.example.com/v1/
paths:
  /orders/{id}:
    get:
      operationId: getOrder
      summary: One order
    delete: {}
  /orders:
    post: {operationId: createOrder}
components:
  schemas:
    Order:
      type: object
      required: [id]
      properties:
        id: {type: string}
        items: {type: array, items: {$ref: '#/components/schemas/Item'}}
    Item:
      allOf:
        - {$ref: '#/components/schemas/Order'}
        - properties: {sku: {type: string}}
"#;

    #[test]
    fn reads_3_0_with_a_base_path_and_schemas() {
        let c = parse(V3.as_bytes(), None).unwrap().unwrap();
        assert_eq!(c.version, "3.0.3");
        assert_eq!(c.bases, ["/v1"]);
        assert_eq!(
            ops(&c),
            ["post /orders", "get /orders/{id}", "delete /orders/{id}"]
        );
        let get = c.operations.iter().find(|o| o.method == "get").unwrap();
        assert_eq!(get.operation_id.as_deref(), Some("getOrder"));
        assert_eq!(get.line, 7);
        let order = c.schemas.iter().find(|s| s.name == "Order").unwrap();
        assert_eq!(
            order.fields,
            [
                SchemaField {
                    name: "id".into(),
                    ty: "string".into(),
                    required: true
                },
                SchemaField {
                    name: "items".into(),
                    ty: "array<ref:Item>".into(),
                    required: false
                },
            ]
        );
        // allOf pulls in the parts' properties.
        let item = c.schemas.iter().find(|s| s.name == "Item").unwrap();
        assert_eq!(item.fields.len(), 3);
    }

    #[test]
    fn reads_swagger_2_and_json() {
        let j = r#"{"swagger":"2.0","basePath":"/api","paths":{"/pets":{"get":{"operationId":"list"}}},
                    "definitions":{"Pet":{"properties":{"name":{"type":"string"}}}}}"#;
        let c = parse(j.as_bytes(), None).unwrap().unwrap();
        assert_eq!(
            (c.version.as_str(), c.bases.as_slice()),
            ("2.0", &["/api".to_owned()][..])
        );
        assert_eq!(ops(&c), ["get /pets"]);
        assert_eq!(c.schemas[0].name, "Pet");
        // 3.1 is the same model.
        let c = parse(b"openapi: 3.1.0\npaths:\n  /a: {get: {}}\n", None)
            .unwrap()
            .unwrap();
        assert_eq!(c.bases, [String::new()]);
        assert_eq!(ops(&c), ["get /a"]);
    }

    #[test]
    fn a_3_0_spec_converted_from_2_0_has_the_same_operations() {
        let v2 = "swagger: '2.0'\nbasePath: /api/v2\npaths:\n  /pets/{id}:\n    get: {operationId: a}\n    put: {operationId: b}\n  /pets:\n    post: {operationId: c}\n";
        let v3 = "openapi: 3.0.0\nservers: [{url: 'https://x.example/api/v2'}]\npaths:\n  /pets/{id}:\n    get: {operationId: a}\n    put: {operationId: b}\n  /pets:\n    post: {operationId: c}\n";
        let (a, b) = (
            parse(v2.as_bytes(), None).unwrap().unwrap(),
            parse(v3.as_bytes(), None).unwrap().unwrap(),
        );
        assert_eq!(a.bases, b.bases);
        assert_eq!(a.operations, b.operations);
    }

    #[test]
    fn a_stripe_sized_spec_reads_within_the_budget() {
        // About 5 MB and 600 operations, most of it descriptions that are never kept.
        let desc = "x".repeat(4000);
        let mut t = String::from("openapi: 3.0.0\npaths:\n");
        for i in 0..600 {
            t.push_str(&format!(
                "  /v1/things/{i}/{{id}}:\n    get:\n      operationId: get{i}\n      description: {desc}\n    post: {{operationId: post{i}}}\n"
            ));
        }
        t.push_str("components:\n  schemas:\n");
        for i in 0..400 {
            t.push_str(&format!(
                "    S{i}:\n      description: {desc}\n      properties:\n        a: {{type: string}}\n"
            ));
        }
        assert!(t.len() > 4_000_000, "{}", t.len());
        let start = Instant::now();
        let c = parse(t.as_bytes(), None).unwrap().unwrap();
        assert_eq!((c.operations.len(), c.schemas.len()), (1200, 400));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn other_documents_are_not_specs() {
        for t in [
            "services:\n  a: {}\n",
            "{\"name\": \"openapi-tools\"}",
            "swagger: \"1.2\"\n",
            "[1, 2",
        ] {
            assert_eq!(parse(t.as_bytes(), None).unwrap(), None, "{t}");
        }
    }

    #[test]
    fn remote_refs_are_never_fetched_and_cycles_stop() {
        let t = "openapi: 3.0.0\npaths:\n  /a: {$ref: 'https://evil.example/x.yaml#/a'}\n  /b: {$ref: '#/components/pathItems/P'}\ncomponents:\n  pathItems:\n    P: {$ref: '#/components/pathItems/Q'}\n    Q: {$ref: '#/components/pathItems/P'}\n";
        let c = parse(
            t.as_bytes(),
            Some(Env {
                dir: "",
                files: &mut None_,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(c.operations.is_empty());
        let reasons: Vec<_> = c.unresolved.iter().map(|u| u.reason.as_str()).collect();
        assert_eq!(reasons, ["cycle", "remote"]);
    }

    #[test]
    fn relative_files_resolve_inside_the_repository_only() {
        let root = "openapi: 3.0.0\npaths:\n  /a: {$ref: 'paths/a.yaml'}\n  /b: {$ref: '../../etc/passwd'}\ncomponents:\n  schemas:\n    A: {$ref: 'schemas.yaml#/A'}\n";
        let mut m = Map(HashMap::from([
            ("api/paths/a.yaml".into(), "get: {operationId: a}\n".into()),
            (
                "api/schemas.yaml".into(),
                "A: {properties: {x: {type: integer}}}\n".into(),
            ),
        ]));
        let c = parse(
            root.as_bytes(),
            Some(Env {
                dir: "api",
                files: &mut m,
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(ops(&c), ["get /a"]);
        assert_eq!(c.files, ["api/paths/a.yaml", "api/schemas.yaml"]);
        assert_eq!(c.schemas[0].fields[0].name, "x");
        assert_eq!(
            c.unresolved,
            [Unresolved {
                target: "../../etc/passwd".into(),
                reason: "outside".into()
            }]
        );
        // Without a reader the files are only listed, as written.
        let c = parse(root.as_bytes(), None).unwrap().unwrap();
        assert!(c.operations.is_empty());
        assert!(c.files.contains(&"paths/a.yaml".to_owned()));
    }

    #[test]
    fn deep_nesting_and_alias_bombs_are_refused_or_bounded() {
        let deep = format!(
            "openapi: 3.0.0\nx: {}1{}\n",
            "[".repeat(100),
            "]".repeat(100)
        );
        assert!(matches!(
            parse(deep.as_bytes(), None),
            Err(Refused::Hostile) | Ok(None)
        ));
        let mut bomb = String::from("openapi: 3.0.0\na0: &a0 [x, x, x, x, x, x, x, x, x]\n");
        for i in 1..12 {
            bomb.push_str(&format!(
                "a{i}: &a{i} [*a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}]\n",
                p = i - 1
            ));
        }
        let t = Instant::now();
        let _ = parse(bomb.as_bytes(), None);
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn facts_round_trip_the_contract() {
        let c = parse(V3.as_bytes(), None).unwrap().unwrap();
        let back = from_facts(&facts(&c)).unwrap();
        assert_eq!(back.operations.len(), c.operations.len());
        assert_eq!(back.schemas, c.schemas);
        assert_eq!(back.bases, c.bases);
    }

    #[test]
    fn path_confinement() {
        assert_eq!(
            join("api", "../shared/x.yaml").as_deref(),
            Some("shared/x.yaml")
        );
        assert_eq!(join("api", "../../x"), None);
        assert_eq!(join("", "/etc/x"), None);
        assert_eq!(join("a", "https://x/y"), None);
    }

    #[test]
    fn names() {
        assert!(
            named("api/openapi.yaml") && named("swagger.json") && named("x/Petstore-OpenAPI.yml")
        );
        assert!(!named("package.json") && !named("openapi.md"));
    }
}
