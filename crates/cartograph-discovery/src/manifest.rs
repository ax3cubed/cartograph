//! `cartograph.yaml`: the committed manifest (C5, B3; IDX-10).
//!
//! A teammate's manifest is untrusted input: it is size-capped, nothing in it is executed or
//! expanded, and its paths are only ever shown and canonicalised, never written to. `index_path`
//! isn't a manifest field at all, so a repo can't redirect where Cartograph writes.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::branch::valid_branch;

pub const FILE_NAME: &str = "cartograph.yaml";
pub const SCHEMA_FILE_NAME: &str = "cartograph.schema.json";
/// The input cap, which with serde_norway's own alias limit bounds an alias bomb.
pub const MAX_MANIFEST: usize = 1024 * 1024;
pub const VERSION: u32 = 1;

/// The whole manifest. Field order here is the key order on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    title = "cartograph.yaml",
    description = "The Cartograph workspace manifest."
)]
pub struct Manifest {
    /// The manifest format. This build reads version 1.
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    pub workspace: Workspace,
    pub repos: Vec<Repo>,
    #[serde(default, skip_serializing_if = "Options::is_empty")]
    pub options: Options,
    /// Services and their paths (shape only here; detection is IDX-20).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
    /// Architecture rules (H6, XS-22).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::rules::Rule>,
    /// The baseline file of known rule violations, relative to the manifest; else
    /// `cartograph.baseline.json` beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    /// OpenAPI and Swagger files by glob (XS-08), besides the ones named `openapi.*` or
    /// `swagger.*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<Contract>,
    /// What Ask and the editor's agent can search (FE-13, board S06 under G7: no embeddings).
    #[serde(default, skip_serializing_if = "Retrieval::is_empty")]
    pub retrieval: Retrieval,
    /// The environment the map shows when none is asked for; else `prod`, else the first (XS-09).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_environment: Option<String>,
    /// Named environments (XS-09): which files give their values, and values set by hand.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environments: BTreeMap<String, Environment>,
    /// Folders of subprocess adapters (XS-24), relative to this file and inside it. Naming one
    /// runs nothing: each needs `cartograph adapters allow <name>` on every machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<String>,
}

/// One environment of `environments:` (XS-09).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Environment {
    /// Globs over the files that give this environment its values, as the index sees paths
    /// (`deploy/prod/**`). They win over the environment a file's name implies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// Values set by hand, key to value: what "Save values" writes. Only a URL, `host[:port]` or
    /// a short identifier is kept; anything else is ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,
}

/// The kinds of text search indexes for Ask and MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    /// Node cards: name, path, summary and main links.
    Cards,
    /// Function and class bodies.
    Symbols,
    /// Code changes.
    Changes,
    /// API operations and event channels.
    Contracts,
    /// READMEs, ADRs and docs.
    Docs,
    /// Commit messages without their changes.
    CommitMessages,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Retrieval {
    /// The kinds searched; absent means all but commit messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<ChunkKind>>,
    /// Leave anything that looks like a key or password out of search and prompts. On when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scan: Option<bool>,
}

impl Retrieval {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub name: String,
    /// The .code-workspace file this was resolved from, relative to the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Repo {
    pub name: String,
    /// The repo's top level, relative to the manifest; `../` is allowed (C5).
    pub path: String,
    /// The display name, from `folders[].name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// `remote.origin.url`, with any credentials removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// The indexed branch; overrides the rest of the C3 ladder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// Only these folders, relative to the repo, produce nodes and links. Empty is the whole repo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Options {
    /// Globs left out of the index, from `files.exclude`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Files larger than this are not parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_bytes: Option<u64>,
}

impl Options {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// A spec the index reads (XS-08).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Contract {
    /// A glob relative to each repo's root: `docs/api/*.yaml`. It picks among YAML and JSON files.
    pub path: String,
    /// The service that owns the specs it matches; absent, the service whose root holds each file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Service {
    pub name: String,
    /// The repo's `name`.
    pub repo: String,
    /// A folder in the repo; the whole repo when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// A manifest that didn't validate: where, what, and the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// 1-based; 0 when the problem has no single place (a missing key).
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub fix: String,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}, column {}: ", self.line, self.column)?;
        }
        f.write_str(&self.message)
    }
}

fn invalid(message: impl Into<String>, fix: impl Into<String>) -> Invalid {
    Invalid {
        line: 0,
        column: 0,
        message: message.into(),
        fix: fix.into(),
    }
}

/// A parsed manifest and its warnings (unknown keys, so an older binary tolerates additions).
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub manifest: Manifest,
    pub warnings: Vec<String>,
}

/// The JSON Schema of [`Manifest`], as shipped in the binary and written beside the data folder.
pub fn schema_json() -> String {
    let mut s = serde_json::to_string_pretty(&schemars::schema_for!(Manifest))
        .expect("a schema serialises");
    s.push('\n');
    s
}

/// Parses and validates the text of a manifest.
pub fn parse(text: &str) -> Result<Parsed, Invalid> {
    if text.len() > MAX_MANIFEST {
        return Err(invalid(
            format!("the manifest is {} bytes; the limit is 1 MB", text.len()),
            "remove what isn't needed; cartograph.yaml lists repos and services, not data",
        ));
    }
    let located = |e: serde_norway::Error, fix: &str| {
        let (line, column) = e.location().map_or((0, 0), |l| (l.line(), l.column()));
        Invalid {
            line,
            column,
            message: e.to_string(),
            fix: fix.into(),
        }
    };
    let value: serde_json::Value = serde_norway::from_str(text)
        .map_err(|e| located(e, "fix the YAML syntax at that place"))?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {}
        Some(v) => {
            return Err(invalid(
                format!("cartograph.yaml is version {v}; this cartograph reads version {VERSION}"),
                "upgrade cartograph to a release that reads this version",
            ));
        }
        None => {
            return Err(invalid(
                "cartograph.yaml has no `version: 1`",
                "add `version: 1` as the first key",
            ));
        }
    }
    let manifest: Manifest = serde_norway::from_str(text)
        .map_err(|e| located(e, "compare the key with cartograph.schema.json"))?;
    validate(&manifest)?;
    let mut warnings = Vec::new();
    let known = serde_json::to_value(&manifest).expect("a manifest serialises");
    unknown_keys(&value, &known, "", &mut warnings);
    Ok(Parsed { manifest, warnings })
}

/// Reads and parses a manifest file.
pub fn read(path: &Path) -> Result<Parsed, crate::DiscoveryError> {
    let bad = |e: Invalid| crate::DiscoveryError::Manifest {
        path: path.display().to_string(),
        invalid: e,
    };
    let meta = std::fs::metadata(path).map_err(|e| {
        bad(invalid(
            e.to_string(),
            "check the file exists and is readable",
        ))
    })?;
    // The manifest is committed, so it is read as untrusted: only a regular file (a link to a
    // device or a pipe reports size 0 and never ends), and never more than the limit.
    if !meta.is_file() {
        return Err(bad(invalid(
            "it isn't a regular file",
            "make cartograph.yaml a plain file",
        )));
    }
    let len = meta.len();
    if len > MAX_MANIFEST as u64 {
        return Err(bad(invalid(
            format!("the manifest is {len} bytes; the limit is 1 MB"),
            "remove what isn't needed",
        )));
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_MANIFEST as u64 + 1).read_to_string(&mut text))
        .map_err(|e| bad(invalid(e.to_string(), "save the file as UTF-8")))?;
    parse(&text).map_err(bad)
}

/// Keys in what was written that the types don't know, as `repos[0].colour`.
fn unknown_keys(
    raw: &serde_json::Value,
    known: &serde_json::Value,
    at: &str,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (raw, known) {
        (Value::Object(r), Value::Object(k)) => {
            for (key, v) in r {
                let here = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                match k.get(key) {
                    Some(kv) => unknown_keys(v, kv, &here, out),
                    // A key the writer would leave out when empty (`scopes: []`) is known.
                    None if is_emptyish(v) => {}
                    None => out.push(format!("unknown key `{here}` is ignored")),
                }
            }
        }
        (Value::Array(r), Value::Array(k)) if at != "rules" => {
            for (i, (rv, kv)) in r.iter().zip(k).enumerate() {
                unknown_keys(rv, kv, &format!("{at}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn is_emptyish(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// A relative path that stays a plain path: no NUL, no `\`, not absolute, no `..` unless allowed.
fn plain_relative(p: &str, allow_up: bool) -> bool {
    !p.is_empty()
        && !p.contains('\0')
        && !p.contains('\\')
        && !Path::new(p).is_absolute()
        && Path::new(p).components().all(|c| match c {
            Component::Normal(_) | Component::CurDir => true,
            Component::ParentDir => allow_up,
            _ => false,
        })
}

fn validate(m: &Manifest) -> Result<(), Invalid> {
    if m.workspace.name.trim().is_empty() {
        return Err(invalid(
            "workspace.name is empty",
            "give the workspace a name",
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    for (i, r) in m.repos.iter().enumerate() {
        if r.name.is_empty() || r.name.contains(['/', '\\', '\0']) {
            return Err(invalid(
                format!("repos[{i}].name `{}` isn't a plain name", r.name),
                "use the repo's folder name",
            ));
        }
        if !names.insert(r.name.as_str()) {
            return Err(invalid(
                format!("repos[{i}].name `{}` appears twice", r.name),
                "give each repo its own name",
            ));
        }
        // Absolute paths are allowed: the spec's own example names /Users/me/src/shared-protos.
        if r.path.contains('\0')
            || !(Path::new(&r.path).is_absolute() || plain_relative(&r.path, true))
        {
            return Err(invalid(
                format!("repos[{i}].path `{}` isn't a usable path", r.path),
                "write the path relative to cartograph.yaml, with / separators",
            ));
        }
        if let Some(b) = r.default_branch.as_deref().filter(|b| !valid_branch(b)) {
            return Err(invalid(
                format!("repos[{i}].default_branch `{b}` isn't a valid branch name"),
                "write the branch name as `git branch` shows it",
            ));
        }
        if let Some(s) = r.scopes.iter().find(|s| !plain_relative(s, false)) {
            return Err(invalid(
                format!("repos[{i}].scopes has `{s}`, which leaves the repo"),
                "write each scope relative to the repo, without ..",
            ));
        }
    }
    for (i, s) in m.services.iter().enumerate() {
        if !names.contains(s.repo.as_str()) {
            return Err(invalid(
                format!("services[{i}].repo `{}` isn't one of repos[].name", s.repo),
                "name a repo listed under repos",
            ));
        }
        if let Some(p) = s.path.as_deref().filter(|p| !plain_relative(p, false)) {
            return Err(invalid(
                format!("services[{i}].path `{p}` leaves the repo"),
                "write the path relative to the repo, without ..",
            ));
        }
    }
    if let Some(b) = m.baseline.as_deref().filter(|p| !plain_relative(p, false)) {
        return Err(invalid(
            format!("baseline `{b}` leaves the manifest's folder"),
            "write the path relative to cartograph.yaml, without ..",
        ));
    }
    crate::rules::validate(&m.rules).map_err(|(what, fix)| invalid(what, fix))
}

/// The manifest as YAML: one comment, then the keys in a fixed order. No `yaml-language-server`
/// schema line: the file is committed, and the only schema path we could write is this machine's
/// data folder, which would differ for every teammate (and a URL would have editors fetch it).
pub fn to_yaml(m: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("# Written by `cartograph init`. Commit it, so teammates index the same repos.\n");
    out.push_str(&serde_norway::to_string(m).expect("a manifest serialises"));
    out
}

/// A hash over the parts of the manifest that change facts: repos, branches, scopes, excludes and
/// the size cap. Paths, remotes and names shown to people don't count, so two machines with the
/// same manifest and commits get the same hash (IDX-17, IDX-21 compare it).
pub fn config_hash(m: &Manifest) -> String {
    let mut repos: Vec<_> = m
        .repos
        .iter()
        .map(|r| {
            let mut scopes = r.scopes.clone();
            scopes.sort();
            serde_json::json!([r.name, r.default_branch, scopes])
        })
        .collect();
    repos.sort_by_key(|v| v.to_string());
    let mut exclude = m.options.exclude.clone();
    exclude.sort();
    let canon = serde_json::json!({
        "repos": repos,
        "exclude": exclude,
        "max_file_bytes": m.options.max_file_bytes,
    });
    let digest = Sha256::digest(canon.to_string().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// C7, "Split" on board S4: adds a `services:` entry to the manifest's text and returns the new
/// text with the diff. The edit is textual, so comments and the rest of the file stay as they are:
/// the entry goes at the end of the `services:` block (or a new block at the end), in the block's
/// indentation. The result is parsed again and must be the old manifest plus the entry.
pub fn add_service(text: &str, service: &Service) -> Result<(String, String), Invalid> {
    let old = parse(text)?.manifest;
    if old
        .services
        .iter()
        .any(|s| s.name == service.name && s.repo == service.repo)
    {
        return Err(invalid(
            format!(
                "{} already lists the service {}",
                service.repo, service.name
            ),
            "choose another name",
        ));
    }
    let entry = serde_norway::to_string(&vec![service.clone()]).expect("a service serialises");
    let lines: Vec<&str> = text.lines().collect();
    let top = |l: &str| !l.is_empty() && !l.starts_with([' ', '\t', '#', '-']);
    let at = lines
        .iter()
        .position(|l| l.starts_with("services:") && l["services:".len()..].trim_start().is_empty());
    let (mut out, added_at, added): (Vec<String>, usize, Vec<String>) = match at {
        Some(i) => {
            // The block runs to the next top-level key; the entry goes after its last item line.
            let end = lines[i + 1..]
                .iter()
                .position(|l| top(l))
                .map_or(lines.len(), |k| i + 1 + k);
            let last = (i + 1..end)
                .rev()
                .find(|k| !lines[*k].trim().is_empty() && !lines[*k].trim_start().starts_with('#'))
                .unwrap_or(i);
            let indent: String = lines[i + 1..end]
                .iter()
                .find(|l| l.trim_start().starts_with('-'))
                .map(|l| l.chars().take_while(|c| *c == ' ').collect())
                .unwrap_or_default();
            let added: Vec<String> = entry.lines().map(|l| format!("{indent}{l}")).collect();
            let mut out: Vec<String> = lines[..=last].iter().map(|l| l.to_string()).collect();
            out.extend(added.iter().cloned());
            out.extend(lines[last + 1..].iter().map(|l| l.to_string()));
            (out, last + 1, added)
        }
        None => {
            let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            let replaced = out
                .iter()
                .position(|l| l.split('#').next().unwrap_or("").trim() == "services: []");
            let mut added = vec!["services:".to_owned()];
            added.extend(entry.lines().map(str::to_owned));
            let at = match replaced {
                Some(k) => {
                    out.remove(k);
                    k
                }
                None => out.len(),
            };
            for (n, l) in added.iter().enumerate() {
                out.insert(at + n, l.clone());
            }
            (out, at, added)
        }
    };
    out.push(String::new());
    let new = out.join("\n");
    let mut want = old;
    want.services.push(service.clone());
    match parse(&new) {
        Ok(p) if p.manifest == want => {}
        _ => {
            return Err(invalid(
                "the services: block couldn't be edited in place",
                "add the entry to cartograph.yaml by hand",
            ));
        }
    }
    let mut diff = format!("@@ -{added_at},0 +{},{} @@\n", added_at + 1, added.len());
    for l in &added {
        diff.push('+');
        diff.push_str(l);
        diff.push('\n');
    }
    Ok((new, diff))
}

/// `to` relative to the folder `from`, with `/` separators and `../` where needed. Both canonical.
/// An absolute path when they share no root (another drive on Windows).
pub fn relative_path(from: &Path, to: &Path) -> String {
    let (a, b): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return to.display().to_string();
    }
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_owned(), a.len() - common).collect();
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// Where a repo of the manifest at `manifest_dir` is on disk.
pub fn repo_dir(manifest_dir: &Path, r: &Repo) -> PathBuf {
    manifest_dir.join(&r.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "version: 1
workspace:
  name: acme
  file: acme.code-workspace
repos:
- name: billing-api
  path: billing-api
  remote: https://github.com/acme/billing-api.git
  default_branch: main
- name: platform
  path: ../platform
  default_branch: main
  scopes:
  - packages/auth
  - packages/events
options:
  exclude:
  - '**/dist'
services:
- name: auth
  repo: platform
  path: packages/auth
";

    fn err(text: &str) -> Invalid {
        parse(text).unwrap_err()
    }

    /// C7: the diff adds the entry and nothing else, in the block's indentation.
    #[test]
    fn a_split_adds_one_entry_and_nothing_else() {
        let svc = Service {
            name: "events".into(),
            repo: "platform".into(),
            path: Some("packages/events".into()),
            tags: vec![],
        };
        let (new, diff) = add_service(GOOD, &svc).unwrap();
        assert_eq!(
            diff,
            "@@ -22,0 +23,3 @@\n+- name: events\n+  repo: platform\n+  path: packages/events\n"
        );
        let old: Vec<&str> = GOOD.lines().collect();
        let mut kept: Vec<&str> = new.lines().collect();
        kept.drain(22..25);
        assert_eq!(kept, old, "every other line is untouched");
        assert!(add_service(&new, &svc).is_err(), "twice is refused");
        // No services block yet: one is added at the end.
        let bare = GOOD.split("services:").next().unwrap();
        let (new, diff) = add_service(bare, &svc).unwrap();
        assert!(new.starts_with(bare));
        assert!(diff.contains("+services:\n"));
        // An indented block keeps its indentation.
        let indented = GOOD.replace(
            "- name: auth\n  repo: platform\n  path: packages/auth",
            "  - name: auth\n    repo: platform\n    path: packages/auth",
        );
        let (new, _) = add_service(&indented, &svc).unwrap();
        assert!(new.contains("\n  - name: events\n    repo: platform\n"));
    }

    #[test]
    fn round_trip() {
        let p = parse(GOOD).unwrap();
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let again = parse(&to_yaml(&p.manifest)).unwrap();
        assert_eq!(again.manifest, p.manifest);
    }

    #[test]
    fn retrieval_is_a_known_section_with_closed_kinds() {
        let p = parse(&format!(
            "{GOOD}retrieval:\n  include: [cards, changes, commit_messages]\n  secret_scan: false\n"
        ))
        .unwrap();
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(
            p.manifest.retrieval.include,
            Some(vec![
                ChunkKind::Cards,
                ChunkKind::Changes,
                ChunkKind::CommitMessages
            ])
        );
        assert_eq!(p.manifest.retrieval.secret_scan, Some(false));
        assert!(!to_yaml(&parse(GOOD).unwrap().manifest).contains("retrieval"));
        assert!(err(&format!("{GOOD}retrieval:\n  include: [embeddings]\n")).line > 0);
    }

    #[test]
    fn version_2_is_refused_with_an_upgrade_message() {
        let e = err(&GOOD.replace("version: 1", "version: 2"));
        assert!(e.message.contains("version 2"));
        assert!(e.fix.contains("upgrade"));
        assert!(
            err(&GOOD.replace("version: 1\n", ""))
                .fix
                .contains("version: 1")
        );
    }

    #[test]
    fn an_unknown_key_is_a_warning() {
        let p = parse(&GOOD.replace(
            "  default_branch: main\n- name: platform",
            "  colour: red\n  default_branch: main\n- name: platform",
        ))
        .unwrap();
        assert_eq!(p.warnings, ["unknown key `repos[0].colour` is ignored"]);
        let p = parse(&format!("{GOOD}telemetry: true\n")).unwrap();
        assert_eq!(p.warnings, ["unknown key `telemetry` is ignored"]);
    }

    #[test]
    fn errors_carry_line_and_column() {
        let e = err(&GOOD.replace(
            "  default_branch: main\n- name: platform",
            "  default_branch: [main]\n- name: platform",
        ));
        assert_eq!(e.line, 9, "{e}");
        assert!(e.column > 0);
        let e = err("version: 1\nworkspace: {name: a\n");
        assert!(e.line > 0, "{e}");
    }

    #[test]
    fn negative_fixtures() {
        for (text, needle) in [
            (GOOD.replace("name: billing-api", "name: platform"), "twice"),
            (
                GOOD.replace("- packages/auth", "- ../../etc"),
                "leaves the repo",
            ),
            (
                GOOD.replace(
                    "default_branch: main\n- name: platform",
                    "default_branch: a..b\n- name: platform",
                ),
                "branch",
            ),
            (
                GOOD.replace("repo: platform", "repo: nowhere"),
                "isn't one of",
            ),
            (GOOD.replace("name: acme", "name: ''"), "workspace.name"),
            (
                GOOD.replace("path: billing-api", "path: \"a\\\\b\""),
                "path",
            ),
        ] {
            let e = err(&text);
            assert!(e.message.contains(needle), "{needle}: {e}");
            assert!(!e.fix.is_empty());
        }
    }

    #[test]
    fn alias_bombs_and_big_files_are_bounded() {
        let mut bomb = String::from("version: 1\na: &a [x,x,x,x,x,x,x,x,x,x]\n");
        for i in 0..12 {
            let prev = if i == 0 {
                "a".to_owned()
            } else {
                format!("a{}", i - 1)
            };
            bomb.push_str(&format!("a{i}: &a{i} [*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev},*{prev}]\n"));
        }
        assert!(parse(&bomb).is_err());
        assert!(parse(&"#".repeat(MAX_MANIFEST + 1)).is_err());
    }

    #[test]
    fn the_hash_ignores_paths_and_remotes_but_not_branches() {
        let m = parse(GOOD).unwrap().manifest;
        let mut moved = m.clone();
        moved.repos[0].path = "../elsewhere/billing-api".into();
        moved.repos[0].remote = None;
        moved.repos.reverse();
        assert_eq!(config_hash(&m), config_hash(&moved));
        let mut other = m.clone();
        other.repos[0].default_branch = Some("develop".into());
        assert_ne!(config_hash(&m), config_hash(&other));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative_path(Path::new("/a/b"), Path::new("/a/b/c")), "c");
        assert_eq!(
            relative_path(Path::new("/a/b"), Path::new("/a/x/y")),
            "../x/y"
        );
        assert_eq!(relative_path(Path::new("/a/b"), Path::new("/a/b")), ".");
    }

    #[test]
    fn the_schema_file_is_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_FILE_NAME);
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(&path, schema_json()).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            schema_json(),
            "run UPDATE_SCHEMA=1 cargo test -p cartograph-discovery the_schema_file_is_current"
        );
    }

    #[test]
    fn a_written_manifest_validates_in_a_third_party_validator() {
        let schema: serde_json::Value = serde_json::from_str(&schema_json()).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let m = parse(GOOD).unwrap().manifest;
        let yaml = to_yaml(&m);
        let as_json: serde_json::Value = serde_norway::from_str(&yaml).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&as_json)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let bad = serde_json::json!({"version": 1, "workspace": {}, "repos": []});
        assert!(!validator.is_valid(&bad));
    }

    proptest::proptest! {
        // ponytail: a property test stands in for the YAML fuzz target until cargo-fuzz (HD-06).
        #[test]
        fn the_reader_never_panics(s in "\\PC{0,300}") {
            let _ = parse(&s);
        }

        #[test]
        fn parse_write_parse_is_stable(
            names in proptest::collection::btree_set("[a-z][a-z0-9-]{0,8}", 1..5),
            branch in "[a-z][a-z0-9/-]{0,10}[a-z]",
        ) {
            let m = Manifest {
                version: 1,
                workspace: Workspace { name: "w".into(), file: None },
                repos: names.iter().map(|n| Repo {
                    name: n.clone(), path: format!("../{n}"), display: None, remote: None,
                    default_branch: valid_branch(&branch).then(|| branch.clone()),
                    scopes: vec![],
                }).collect(),
                options: Options::default(),
                services: vec![], rules: vec![], baseline: None, contracts: vec![], retrieval: Default::default(),
                default_environment: None, environments: Default::default(), adapters: vec![],
            };
            let again = parse(&to_yaml(&m)).unwrap().manifest;
            proptest::prop_assert_eq!(again, m);
        }
    }
}
