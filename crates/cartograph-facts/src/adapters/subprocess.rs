//! Subprocess adapters (XS-24, H4b): an adapter that is a program, for a language the built-in query
//! adapters don't read. This is the one place Cartograph runs code a repository chose, so every
//! step is conservative:
//!
//! - A manifest only *declares* an adapter. It runs after `cartograph adapters allow <name>`, which
//!   stores [`Declared::hash`] in the per-user registry; any change to the adapter folder, or to the
//!   path its command resolves to, asks again. Nothing here approves anything.
//! - The folder is inside the manifest's folder, with no symlink in it.
//! - The process runs in a sandbox with no network, no file writes and no reads but the system's
//!   runtime and its own folder (macOS `sandbox-exec`, Linux `bwrap`). With no sandbox (Windows) it
//!   is refused unless the user passes `--unsandboxed-adapters`.
//! - Its environment is scrubbed, its working folder is empty and temporary, and the blob arrives on
//!   stdin, so it needs no access to the repository.
//! - Limits: 10 s wall per blob, CPU and memory `ulimit`s, 4 MiB of output per blob, 64 KiB per line,
//!   10k facts per blob. A breach, or a line that doesn't validate, kills the process; the blob's
//!   facts are dropped and not cached, and the adapter doesn't run again that run.
//!
//! # Protocol (NDJSON, one process per run)
//!
//! Cartograph writes one line per blob:
//! `{"type":"extract","id":7,"blob":"<hex>","path":"src/a.py","content":"<the file, UTF-8>"}`.
//! The adapter answers with any number of fact lines and then a done line:
//! `{"type":"fact","id":7,"blob":"<hex>","path":"src/a.py","kind":"route","span":[1,0,1,9],`
//! `"attrs":{"method":"GET","path":"/x"},"evidence":{"type":"literal","text":"..."}}` and
//! `{"type":"done","id":7}`. `id`, `blob` and `path` must echo the request.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{AdapterFacts, LoadError, glob, valid_name};
use crate::cache::FactCache;
use crate::schema::BlobFact;

pub const WALL: Duration = Duration::from_secs(10);
pub const MAX_OUTPUT: usize = 4 << 20;
pub const MAX_LINE: usize = 64 << 10;
pub const MAX_FACTS: usize = 10_000;
/// The largest blob sent: the parse cap.
pub const MAX_CONTENT: usize = cartograph_parse::MAX_BLOB_BYTES as usize;
const MAX_TOML: u64 = 64 << 10;
const MAX_FILES: usize = 256;
const MAX_FILE: u64 = 4 << 20;
const MAX_FOLDER: u64 = 16 << 20;
/// Where a bare `command` is looked for, and the only `PATH` the adapter sees.
const SAFE_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Toml {
    name: String,
    version: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    files: Vec<String>,
}

/// An adapter a manifest names, read and checked, not yet approved.
#[derive(Debug, Clone)]
pub struct Declared {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    /// Where `command` resolved to: part of what the user approves.
    pub command: PathBuf,
    pub args: Vec<String>,
    files: Vec<Regex>,
    /// SHA-256 over `adapter.toml`, the resolved command path and every file of the folder.
    pub hash: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn err(s: impl Into<String>) -> LoadError {
    LoadError(s.into())
}

/// A folder name from a manifest: relative, no `..`, nothing a profile or a shell could misread.
fn safe_relative(p: &str) -> bool {
    !p.is_empty()
        && !p.contains(['\0', '\\', '"', '(', ')', '\n'])
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// `command` as a path: inside `dir` when it names a folder, else the first match on [`SAFE_PATH`].
fn resolve_command(dir: &Path, command: &str) -> Result<PathBuf, LoadError> {
    let found = if command.contains('/') {
        if !safe_relative(command) {
            return Err(err("command must stay inside the adapter folder"));
        }
        let p = dir
            .join(command)
            .canonicalize()
            .map_err(|e| err(format!("command: {e}")))?;
        if !p.starts_with(dir) {
            return Err(err("command must stay inside the adapter folder"));
        }
        p
    } else {
        if command.is_empty() || command.contains(['\0', '"', '(', ')', '\\']) {
            return Err(err("bad command"));
        }
        SAFE_PATH
            .split(':')
            .map(|d| Path::new(d).join(command))
            .find(|p| p.is_file())
            .ok_or_else(|| err(format!("command {command} not found in {SAFE_PATH}")))?
    };
    Ok(found)
}

impl Declared {
    /// The adapter in `folder` of the manifest folder `root`.
    pub fn load(root: &Path, folder: &str) -> Result<Self, LoadError> {
        if !safe_relative(folder) {
            return Err(err(format!(
                "adapter folder {folder} must be inside the repository"
            )));
        }
        let root = root.canonicalize().map_err(|e| err(e.to_string()))?;
        let dir = root
            .join(folder)
            .canonicalize()
            .map_err(|e| err(format!("{folder}: {e}")))?;
        if !dir.starts_with(&root) || dir == root {
            return Err(err(format!(
                "adapter folder {folder} must be inside the repository"
            )));
        }
        let toml_path = dir.join("adapter.toml");
        let meta =
            std::fs::symlink_metadata(&toml_path).map_err(|e| err(format!("adapter.toml: {e}")))?;
        if !meta.is_file() || meta.len() > MAX_TOML {
            return Err(err("adapter.toml must be a file under 64 KiB"));
        }
        let text = std::fs::read_to_string(&toml_path).map_err(|e| err(e.to_string()))?;
        let t: Toml = toml::from_str(&text).map_err(|e| err(e.to_string()))?;
        if !valid_name(&t.name) || t.version.is_empty() || t.version.len() > 32 {
            return Err(err(format!("bad name or version: {}", t.name)));
        }
        if t.files.is_empty() || t.files.len() > 16 || t.args.len() > 32 {
            return Err(err("files takes 1 to 16 globs, args up to 32"));
        }
        if t.args.iter().any(|a| a.contains('\0')) {
            return Err(err("bad args"));
        }
        let command = resolve_command(&dir, &t.command)?;
        let files = t.files.iter().map(|g| glob(g)).collect::<Result<_, _>>()?;
        let hash = folder_hash(&dir, &command)?;
        Ok(Self {
            name: t.name,
            version: t.version,
            dir,
            command,
            args: t.args,
            files,
            hash,
        })
    }

    pub fn applies(&self, path: &str) -> bool {
        self.files.iter().any(|f| f.is_match(path))
    }

    /// The cache row name: it never collides with a built-in adapter's.
    fn cache_key(&self) -> String {
        format!("subprocess-{}", self.name)
    }
}

/// The hash the user approves. A symlink, or a folder past the caps, is an error: it can't be read
/// as a whole.
fn folder_hash(dir: &Path, command: &Path) -> Result<String, LoadError> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| err(e.to_string()))? {
            let e = e.map_err(|e| err(e.to_string()))?;
            let t = e.file_type().map_err(|e| err(e.to_string()))?;
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                files.push(e.path());
            } else {
                return Err(err(format!(
                    "{}: symlinks are not allowed in an adapter folder",
                    e.path().display()
                )));
            }
        }
    }
    files.sort();
    if files.len() > MAX_FILES {
        return Err(err("adapter folder has more than 256 files"));
    }
    let mut h = Sha256::new();
    h.update(b"command\0");
    h.update(command.as_os_str().as_encoded_bytes());
    let mut total = 0u64;
    for f in files {
        let n = std::fs::metadata(&f).map_err(|e| err(e.to_string()))?.len();
        total += n;
        if n > MAX_FILE || total > MAX_FOLDER {
            return Err(err("adapter folder is too large"));
        }
        let rel = f.strip_prefix(dir).unwrap_or(&f);
        h.update(b"\0file\0");
        h.update(rel.as_os_str().as_encoded_bytes());
        h.update(b"\0");
        h.update(std::fs::read(&f).map_err(|e| err(e.to_string()))?);
    }
    Ok(hex(&h.finalize()))
}

/// How a process is confined on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sandbox {
    /// macOS `sandbox-exec` with a deny-default profile.
    Seatbelt,
    /// Linux bubblewrap: no namespaces shared (so no network), read-only binds of the runtime.
    Bwrap(PathBuf),
    None,
}

impl Sandbox {
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file() {
            return Self::Seatbelt;
        }
        if cfg!(target_os = "linux")
            && let Some(p) = ["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
        {
            return Self::Bwrap(p);
        }
        Self::None
    }
}

/// The `sandbox-exec` profile: everything denied but running the program, reading the system's
/// runtime, the adapter's folder and the empty working folder. No network, no writes.
fn profile(dir: &Path, tmp: &Path) -> String {
    let mut p = String::from(
        "(version 1)\n(deny default)\n(allow process-exec)\n(allow process-fork)\n\
         (allow sysctl-read)\n(allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\"))\n\
         (allow file-read-metadata)\n(allow file-write* (literal \"/dev/null\"))\n(allow file-read* (literal \"/\")",
    );
    for s in [
        "/usr",
        "/bin",
        "/sbin",
        "/System",
        "/Library",
        "/private/etc",
        "/opt/homebrew",
        "/dev",
        "/private/var/db/timezone",
        "/Applications/Xcode.app",
        "/Library/Developer",
    ] {
        p.push_str(&format!(" (subpath \"{s}\")"));
    }
    for s in [dir, tmp] {
        p.push_str(&format!(" (subpath \"{}\")", s.display()));
    }
    p.push_str(")\n");
    p
}

/// The program and arguments that run `d` under `sb` (`None`: unsandboxed). `sh` applies the CPU and
/// memory limits, then becomes the adapter.
fn launch(sb: &Sandbox, d: &Declared, tmp: &Path) -> Command {
    let inner = [
        "/bin/sh",
        "-c",
        "ulimit -t 20 2>/dev/null; ulimit -v 4194304 2>/dev/null; exec \"$0\" \"$@\"",
    ];
    let mut c;
    match sb {
        Sandbox::Seatbelt => {
            c = Command::new("/usr/bin/sandbox-exec");
            c.arg("-p").arg(profile(&d.dir, tmp));
            c.args(inner);
        }
        Sandbox::Bwrap(bwrap) => {
            c = Command::new(bwrap);
            c.args([
                "--die-with-parent",
                "--new-session",
                "--unshare-all",
                "--proc",
                "/proc",
            ]);
            c.args(["--dev", "/dev", "--tmpfs", "/tmp"]);
            for s in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/opt"] {
                c.args(["--ro-bind-try", s, s]);
            }
            c.args(["--ro-bind-try", "/etc/ld.so.cache", "/etc/ld.so.cache"]);
            c.arg("--ro-bind").arg(&d.dir).arg(&d.dir);
            c.args(["--chdir", "/tmp", "--"]);
            c.args(inner);
        }
        Sandbox::None => {
            c = Command::new(inner[0]);
            c.args(&inner[1..]);
        }
    }
    // The working folder is empty, so an argument that names a file of the adapter's folder is
    // made absolute.
    c.arg(&d.command).args(d.args.iter().map(|a| {
        let p = d.dir.join(a);
        if safe_relative(a) && p.exists() {
            p.into_os_string()
        } else {
            a.into()
        }
    }));
    c.env_clear()
        .env("PATH", SAFE_PATH)
        .env("LANG", "C.UTF-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .current_dir(tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    c
}

const STOPPED: &str = "stopped earlier in this run";

type Line = Result<Vec<u8>, String>;

/// Reads lines of at most [`MAX_LINE`] bytes; the bound channel is the back-pressure that keeps a
/// flood out of memory.
fn read_lines(out: impl Read, tx: SyncSender<Line>) {
    let mut r = BufReader::new(out);
    loop {
        let mut buf = Vec::new();
        let n = (&mut r)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf);
        let msg = match n {
            Ok(0) => Err("exited".to_owned()),
            Ok(_) if buf.last() == Some(&b'\n') => {
                buf.pop();
                Ok(buf)
            }
            Ok(_) if buf.len() > MAX_LINE => Err("a line over 64 KiB".to_owned()),
            Ok(_) => Err("exited in the middle of a line".to_owned()),
            Err(e) => Err(e.to_string()),
        };
        let stop = msg.is_err();
        if tx.send(msg).is_err() || stop {
            return;
        }
    }
}

/// A running adapter.
struct Proc {
    child: Child,
    to: SyncSender<Vec<u8>>,
    from: Receiver<Line>,
    id: u64,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Proc {
    fn start(mut c: Command) -> Result<Self, String> {
        let mut child = c.spawn().map_err(|e| format!("can't start: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let (to, rx) = sync_channel::<Vec<u8>>(2);
        std::thread::spawn(move || {
            for m in rx {
                if stdin.write_all(&m).and_then(|()| stdin.flush()).is_err() {
                    return;
                }
            }
        });
        let (tx, from) = sync_channel(64);
        std::thread::spawn(move || read_lines(out, tx));
        Ok(Self {
            child,
            to,
            from,
            id: 0,
        })
    }

    /// The facts for one blob, or why the adapter is stopped.
    fn extract(
        &mut self,
        blob: &str,
        path: &str,
        content: &str,
        wall: Duration,
    ) -> Result<Vec<BlobFact>, String> {
        self.id += 1;
        let id = self.id;
        let mut req =
            json!({"type": "extract", "id": id, "blob": blob, "path": path, "content": content})
                .to_string()
                .into_bytes();
        req.push(b'\n');
        let deadline = Instant::now() + wall;
        self.to.send(req).map_err(|_| "exited".to_owned())?;
        let lines = content.split('\n').count() as u64;
        let (mut facts, mut total) = (Vec::new(), 0usize);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.from.recv_timeout(left) {
                Ok(l) => l?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err("timed out".into()),
                Err(_) => return Err("exited".into()),
            };
            total += line.len() + 1;
            if total > MAX_OUTPUT {
                return Err("more than 4 MiB of output for one blob".into());
            }
            match parse_line(&line, id, blob, path, lines)? {
                Parsed::Done => return Ok(facts),
                Parsed::Fact(f) => {
                    if facts.len() >= MAX_FACTS {
                        return Err("more than 10000 facts for one blob".into());
                    }
                    facts.push(f);
                }
            }
        }
    }
}

enum Parsed {
    Done,
    Fact(BlobFact),
}

/// One line of the adapter's answer, validated: the request is echoed, the span is inside the
/// file, the kind is in the registry, evidence is capped and secret-masked.
fn parse_line(line: &[u8], id: u64, blob: &str, path: &str, lines: u64) -> Result<Parsed, String> {
    let Value::Object(mut o) =
        serde_json::from_slice(line).map_err(|e| format!("not JSON: {e}"))?
    else {
        return Err("a line that is not an object".into());
    };
    let ty = o.remove("type");
    if o.remove("id").and_then(|v| v.as_u64()) != Some(id) {
        return Err("a line for another request".into());
    }
    if ty.as_ref().and_then(Value::as_str) == Some("done") {
        return if o.is_empty() {
            Ok(Parsed::Done)
        } else {
            Err("a done line with extra fields".into())
        };
    }
    if ty.as_ref().and_then(Value::as_str) != Some("fact") {
        return Err("an unknown line type".into());
    }
    if o.remove("blob").as_ref().and_then(Value::as_str) != Some(blob) {
        return Err("a fact that names another blob".into());
    }
    if o.remove("path").as_ref().and_then(Value::as_str) != Some(path) {
        return Err("a fact that names another file's path".into());
    }
    if let Some(Value::Object(ev)) = o.get_mut("evidence")
        && let Some(Value::String(t)) = ev.get_mut("text")
    {
        let masked = cartograph_core::secrets::mask(t).into_owned();
        *t = masked.chars().take(crate::schema::MAX_EVIDENCE).collect();
    }
    let fact: BlobFact =
        serde_json::from_value(Value::Object(o)).map_err(|e| format!("invalid fact: {e}"))?;
    let [sl, sc, el, ec] = fact.span.0;
    let (sl, el) = (u64::from(sl), u64::from(el));
    if sl < 1 || sl > el || el > lines || (sl == el && sc > ec) {
        return Err("a span outside the file".into());
    }
    Ok(Parsed::Fact(fact))
}

/// The adapters allowed to run in a workspace, and what was left out and why.
#[derive(Debug, Clone, Default)]
pub struct SubSet {
    adapters: Vec<Declared>,
    /// One line per declared adapter that will not run.
    pub skipped: Vec<String>,
    /// `--unsandboxed-adapters`: run with no sandbox where there is none.
    pub unsandboxed: bool,
    /// The wall limit per blob; [`WALL`] when absent. Tests shorten it.
    pub wall: Option<Duration>,
}

impl SubSet {
    /// `approved(name, hash)` says whether the user's registry holds this exact adapter.
    pub fn new(
        declared: Vec<Result<Declared, String>>,
        approved: impl Fn(&Declared) -> bool,
    ) -> Self {
        let mut s = Self::default();
        for d in declared {
            match d {
                Err(e) => s.skipped.push(format!("adapter not run: {e}")),
                Ok(d) if approved(&d) => s.adapters.push(d),
                Ok(d) => s.skipped.push(format!(
                    "adapter {} not run: not approved on this machine; read {} and run `cartograph adapters allow {}`",
                    d.name,
                    d.dir.join("adapter.toml").display(),
                    d.name
                )),
            }
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }

    /// Part of the `adapters` extractor version: approving or removing an adapter makes the
    /// interface facts again.
    pub fn version(&self) -> String {
        let mut v: Vec<_> = self
            .adapters
            .iter()
            .map(|a| format!("+{}@{}", a.name, a.version))
            .collect();
        v.sort();
        v.concat()
    }

    /// A run's view: processes start on the first blob the cache can't answer.
    pub fn session(&self) -> Session<'_> {
        Session {
            set: self,
            procs: HashMap::new(),
            notes: Vec::new(),
            tmp: None,
        }
    }
}

/// One run's adapter processes, killed when it drops.
pub struct Session<'a> {
    set: &'a SubSet,
    /// By adapter index; `None` after a failure.
    procs: HashMap<usize, Option<Proc>>,
    /// What went wrong, for the sync's report.
    pub notes: Vec<String>,
    tmp: Option<tempfile::TempDir>,
}

impl Session<'_> {
    /// True when no adapter is approved: nothing to ask.
    pub fn is_idle(&self) -> bool {
        self.set.is_empty()
    }

    /// Facts of each adapter that applies to `path`, from the cache or the adapter. `read` gives
    /// the blob's bytes and is called at most once, and only on a miss.
    pub fn extract(
        &mut self,
        cache: &mut FactCache,
        blob: &[u8],
        path: &str,
        read: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Vec<AdapterFacts> {
        let mut out = Vec::new();
        let mut bytes: Option<Option<Vec<u8>>> = None;
        let mut read = Some(read);
        for (i, a) in self.set.adapters.iter().enumerate() {
            if !a.applies(path) {
                continue;
            }
            let key = a.cache_key();
            let facts = if let Some(f) = cache.get_interface(blob, &key, &a.version) {
                f
            } else {
                let b = bytes.get_or_insert_with(|| read.take().and_then(|r| r()));
                let Some(b) = b.as_deref() else { continue };
                let text = std::str::from_utf8(b)
                    .ok()
                    .filter(|_| b.len() <= MAX_CONTENT);
                let Some(text) = text else {
                    // Binary or too big: nothing to say about it, now or later.
                    let _ = cache.put_interface_many([(
                        blob,
                        key.as_str(),
                        a.version.as_str(),
                        &[][..],
                    )]);
                    out.push(AdapterFacts {
                        name: a.name.clone(),
                        version: a.version.clone(),
                        facts: Vec::new(),
                    });
                    continue;
                };
                match self.run(i, &hex(blob), path, text) {
                    Ok(f) => {
                        let _ = cache.put_interface_many([(
                            blob,
                            key.as_str(),
                            a.version.as_str(),
                            f.as_slice(),
                        )]);
                        f
                    }
                    Err(why) => {
                        if why != STOPPED {
                            self.notes
                                .push(format!("adapter {} stopped on {path}: {why}", a.name));
                        }
                        continue;
                    }
                }
            };
            out.push(AdapterFacts {
                name: a.name.clone(),
                version: a.version.clone(),
                facts,
            });
        }
        out
    }

    fn run(
        &mut self,
        i: usize,
        blob: &str,
        path: &str,
        text: &str,
    ) -> Result<Vec<BlobFact>, String> {
        let a = &self.set.adapters[i];
        if !self.procs.contains_key(&i) {
            let sb = Sandbox::detect();
            let started = if sb == Sandbox::None && !self.set.unsandboxed {
                Err(
                    "no sandbox on this system; pass --unsandboxed-adapters to run it anyway"
                        .to_owned(),
                )
            } else {
                let tmp = match &self.tmp {
                    Some(t) => t,
                    None => self
                        .tmp
                        .insert(tempfile::tempdir().map_err(|e| e.to_string())?),
                };
                let tmp = tmp.path().canonicalize().map_err(|e| e.to_string())?;
                Proc::start(launch(&sb, a, &tmp))
            };
            match started {
                Ok(p) => self.procs.insert(i, Some(p)),
                Err(e) => {
                    self.procs.insert(i, None);
                    return Err(e);
                }
            };
        }
        let Some(Some(p)) = self.procs.get_mut(&i) else {
            return Err(STOPPED.into());
        };
        let r = p.extract(blob, path, text, self.set.wall.unwrap_or(WALL));
        if r.is_err() {
            self.procs.insert(i, None);
        }
        r
    }
}

#[cfg(test)]
mod tests;
