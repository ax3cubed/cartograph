//! A real server on a real listener over a fixture index, raw HTTP/1.1 requests, and a JSON
//! Schema checker for the subset schemars emits.

#![allow(dead_code)] // each test binary uses part of this

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_server::{ServeOptions, ServerInfo, bind};
use serde_json::Value;

pub struct Running {
    pub info: ServerInfo,
    pub data: PathBuf,
    pub ws: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    done: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    rt: tokio::runtime::Runtime,
    _dir: Option<tempfile::TempDir>,
}

impl Running {
    /// Stops the server and waits for it; returns the data folder's owner so it outlives this.
    pub fn stop(mut self) -> tempfile::TempDir {
        let _ = self.stop.take().unwrap().send(());
        let done = self.done.take().unwrap();
        self.rt.block_on(done).unwrap().unwrap();
        self._dir.take().unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// A folder holding a fixture repo and a data folder with that repo registered and indexed, the
/// way `cartograph init` does it: discovery's plan and apply, then a sync.
pub fn data_folder() -> (tempfile::TempDir, PathBuf, String) {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let mut f = Fixture::init(&root.join("repo"));
    f.write("package.json", r#"{"name":"root","private":true}"#);
    f.write("src/a.ts", "import { b } from './b';\n");
    f.write("src/b.ts", "export const b = 1;\n");
    f.commit("one");
    f.write(
        "src/c.ts",
        "import a from './a';\nimport { b } from './b';\n",
    );
    f.commit("two");
    let data = root.join("data");
    let ws = register(&data, &f.dir);
    (d, data, ws)
}

/// Registers the workspace in `dir` and syncs it; returns its id.
pub fn register(data: &std::path::Path, dir: &std::path::Path) -> String {
    let plan = cartograph_discovery::setup::plan(dir, None).unwrap();
    let applied = cartograph_discovery::setup::apply(&plan, data).unwrap();
    sync(data, &applied.workspace_id);
    applied.workspace_id
}

/// Syncs a registered workspace in this process, as `cartograph sync` would.
pub fn sync(data: &std::path::Path, ws: &str) {
    let (reg, _) = cartograph_discovery::Registry::load(data).unwrap();
    let entry = reg.workspaces.iter().find(|w| w.id == ws).unwrap();
    let m = cartograph_discovery::manifest::read(&entry.manifest).unwrap();
    let base = entry.manifest.parent().unwrap();
    let w = cartograph_index::sync::Workspace {
        dir: cartograph_core::paths::workspace_dir(data, ws),
        repos: m
            .manifest
            .repos
            .iter()
            .map(|r| cartograph_index::timeline::RepoSpec {
                name: r.name.clone(),
                path: cartograph_discovery::manifest::repo_dir(base, r),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut cache = FactCache::open(&data.join("cache/facts.db")).unwrap();
    cartograph_index::sync::sync(&w, &Default::default(), &mut cache).unwrap();
}

pub fn start() -> Running {
    let (dir, data, ws) = data_folder();
    start_in(dir, data, ws)
}

pub fn start_in(dir: tempfile::TempDir, data: PathBuf, ws: String) -> Running {
    let rt = runtime();
    let server = rt
        .block_on(bind(ServeOptions {
            data_dir: data.clone(),
            host: None,
            port: None,
            home: data.parent().map(PathBuf::from),
        }))
        .unwrap();
    let info = server.info().clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let done = rt.spawn(server.run(async {
        let _ = rx.await;
    }));
    Running {
        info,
        data,
        ws,
        stop: Some(tx),
        done: Some(done),
        rt,
        _dir: Some(dir),
    }
}

pub struct Reply {
    pub status: u16,
    /// Lower-case names.
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }
    pub fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

/// Sends `head` (request line and headers, without the blank line) and reads the whole reply.
pub fn raw(port: u16, head: &str) -> Reply {
    send(port, head, None)
}

/// [`raw`] with a body; `Content-Length` is added.
pub fn send(port: u16, head: &str, body: Option<&str>) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let body = body
        .map(|b| format!("\r\nContent-Length: {}\r\n\r\n{b}", b.len()))
        .unwrap_or_else(|| "\r\n\r\n".into());
    s.write_all(format!("{head}\r\nConnection: close{body}").as_bytes())
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (top, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = top.lines();
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

/// A GET as the app sends it: own Host, own Origin, the token.
pub fn get(r: &Running, path: &str) -> Reply {
    let p = r.info.port;
    raw(
        p,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: {}\r\nAuthorization: Bearer {}",
            r.info.origin(),
            r.info.token
        ),
    )
}

/// Checks `v` against `schema`, resolving `#/components/schemas/…` in `root`. Objects may not
/// carry properties the schema doesn't name: the TypeScript types are generated with
/// additionalProperties false, so an extra field would be a contract break.
pub fn validate(root: &Value, schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    let s = match schema {
        // `true` takes any value (`details`), `false` none.
        Value::Bool(true) => return Ok(()),
        Value::Object(s) => s,
        _ => return Err(format!("{at}: nothing matches {schema}")),
    };
    for k in s.keys() {
        assert!(
            matches!(
                k.as_str(),
                "$ref"
                    | "type"
                    | "properties"
                    | "required"
                    | "additionalProperties"
                    | "items"
                    | "enum"
                    | "const"
                    | "anyOf"
                    | "oneOf"
                    | "minimum"
                    | "maximum"
                    | "minItems"
                    | "maxItems"
                    | "format"
                    | "description"
                    | "default"
                    | "title"
            ),
            "the checker doesn't know the keyword {k}"
        );
    }
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let name = r
            .strip_prefix("#/components/schemas/")
            .ok_or(format!("{at}: ref {r}"))?;
        return validate(root, &root["components"]["schemas"][name], v, at);
    }
    if let Some(t) = s.get("type") {
        let types: Vec<&str> = match t {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let ok = types.iter().any(|t| match *t {
            "null" => v.is_null(),
            "boolean" => v.is_boolean(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            _ => false,
        });
        if !ok {
            return Err(format!("{at}: {v} is not {types:?}"));
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        return Err(format!("{at}: {v} not in {e:?}"));
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        return Err(format!("{at}: {v} != {c}"));
    }
    if let Some(n) = v.as_f64() {
        if s.get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n < m)
        {
            return Err(format!("{at}: {n} under minimum"));
        }
        if s.get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n > m)
        {
            return Err(format!("{at}: {n} over maximum"));
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(alts) = s.get(key).and_then(Value::as_array) {
            let passing = alts
                .iter()
                .filter(|a| validate(root, a, v, at).is_ok())
                .count();
            if passing == 0 || (key == "oneOf" && passing > 1) {
                return Err(format!("{at}: {passing} of {key} match {v}"));
            }
        }
    }
    if let (Some(items), Some(a)) = (s.get("items"), v.as_array()) {
        if s.get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| (a.len() as u64) < m)
            || s.get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|m| a.len() as u64 > m)
        {
            return Err(format!("{at}: {} items", a.len()));
        }
        for (i, x) in a.iter().enumerate() {
            validate(root, items, x, &format!("{at}[{i}]"))?;
        }
    }
    if let Some(o) = v.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        for r in s
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let r = r.as_str().unwrap();
            if !o.contains_key(r) {
                return Err(format!("{at}: missing {r}"));
            }
        }
        for (k, x) in o {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => validate(root, ps, x, &format!("{at}.{k}"))?,
                None => match s.get("additionalProperties") {
                    Some(Value::Bool(true)) => {}
                    Some(a @ Value::Object(_)) => validate(root, a, x, &format!("{at}.{k}"))?,
                    _ if props.is_some() => return Err(format!("{at}: unexpected property {k}")),
                    _ => {}
                },
            }
        }
    }
    Ok(())
}

/// The generated OpenAPI file, as checked in.
pub fn openapi() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cartograph-api/openapi.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// A write as the app sends it: own Host and Origin, the token, a JSON body.
pub fn write(r: &Running, method: &str, path: &str, body: Option<&str>) -> Reply {
    let p = r.info.port;
    let ct = if body.is_some() {
        "\r\nContent-Type: application/json"
    } else {
        ""
    };
    send(
        p,
        &format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: {}\r\nAuthorization: Bearer {}{ct}",
            r.info.origin(),
            r.info.token
        ),
        body,
    )
}

/// `GET /api/v1/events`, read as the app reads it.
pub struct Events(BufReader<TcpStream>);

impl Events {
    pub fn open(r: &Running) -> Self {
        let p = r.info.port;
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        write!(
            s,
            "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: {}\r\nAuthorization: Bearer {}\r\n\r\n",
            r.info.origin(),
            r.info.token
        )
        .unwrap();
        let mut b = BufReader::new(s);
        let mut status = String::new();
        b.read_line(&mut status).unwrap();
        assert!(status.starts_with("HTTP/1.1 200"), "{status}");
        Self(b)
    }

    /// The next event whose name is `want`, skipping others, within `secs`.
    pub fn wait(&mut self, want: &str, secs: u64) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        let (mut event, mut data) = (String::new(), String::new());
        while Instant::now() < end {
            let mut line = String::new();
            match self.0.read_line(&mut line) {
                Ok(0) => panic!("the stream ended"),
                Ok(_) => {}
                Err(_) => continue, // read timeout: look at the clock again
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(e) = line.strip_prefix("event: ") {
                event = e.to_owned();
            } else if let Some(d) = line.strip_prefix("data: ") {
                data = d.to_owned();
            } else if line.is_empty() && !event.is_empty() {
                if event == want {
                    return serde_json::from_str(&data).unwrap();
                }
                event.clear();
            }
        }
        panic!("no {want} event within {secs} s");
    }
}
