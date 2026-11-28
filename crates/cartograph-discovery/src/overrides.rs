//! `cartograph.overrides.yaml`: link corrections, committed beside the manifest (XS-12, 4.7).
//!
//! A teammate's file is untrusted input (F8): it is size-capped, nothing in it is executed or
//! expanded, `reason` is data capped at 500 characters, and an end names a service and a path the
//! index knows, never a place on disk. What an entry means is the index's business
//! (`cartograph_index::corrections`); this module only reads and checks the shape.
//!
//! One entry is one of H3's four verbs (`remove`, `retarget`, `set`, `add`) on the link named by
//! `key`. An end is `service`, `service:path` or `service:path#symbol`.

use std::io::Read;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "cartograph.overrides.yaml";
pub const SCHEMA_FILE_NAME: &str = "cartograph.overrides.schema.json";
/// The input cap, which with serde_norway's own alias limit bounds an alias bomb.
pub const MAX_FILE: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_REASON: usize = 500;

/// The whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "cartograph.overrides.yaml",
    description = "Corrections to the links Cartograph found."
)]
pub struct Overrides {
    /// The file format. This build reads version 1.
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    /// What `cartograph check` does with a correction that matches nothing: fail (the default),
    /// warn or say nothing.
    #[serde(default, skip_serializing_if = "Stale::is_default")]
    pub stale: Stale,
    // `corrections:` with nothing under it is an empty list.
    #[serde(default, deserialize_with = "null_is_empty")]
    #[schemars(length(max = 10000))]
    pub corrections: Vec<Entry>,
}

fn null_is_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Entry>, D::Error> {
    Ok(Option::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stale {
    #[default]
    Error,
    Warn,
    Off,
}

impl Stale {
    fn is_default(&self) -> bool {
        *self == Self::Error
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Off => "off",
        }
    }
}

/// H3's four verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Hide the link.
    Remove,
    /// Rewrite the far end (`key.to`) to `to`.
    Retarget,
    /// Fix the level to `confidence`.
    Set,
    /// Open a link between two files over the time both exist.
    Add,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remove => "remove",
            Self::Retarget => "retarget",
            Self::Set => "set",
            Self::Add => "add",
        }
    }
}

/// Link kinds, as on every wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Imports,
    Calls,
    Http,
    Event,
    Db,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::Http => "http",
            Self::Event => "event",
            Self::Db => "db",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Score 0.95.
    Exact,
    /// Score 0.75.
    Likely,
    /// Score 0.45.
    Guessed,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Likely => "likely",
            Self::Guessed => "guessed",
        }
    }
    /// The midpoint of the level's band.
    pub fn score(self) -> f64 {
        match self {
            Self::Exact => 0.95,
            Self::Likely => 0.75,
            Self::Guessed => 0.45,
        }
    }
}

/// The link an entry is about: kind, both ends and what the link carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub kind: Kind,
    /// `service`, `service:path` or `service:path#symbol`.
    pub from: String,
    /// The far end, in the same form.
    pub to: String,
    /// `GET /orders/{id}`: for kind `http`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// For kind `event`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// For kind `db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
}

impl Key {
    /// The `op`, `topic` or `table` the kind carries.
    pub fn carried(&self) -> Option<&str> {
        self.op
            .as_deref()
            .or(self.topic.as_deref())
            .or(self.table.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub action: Action,
    pub key: Key,
    /// retarget: the new far end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// set: the level to fix; add: the level of the new link (default likely).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Level>,
    /// Why, for the next reader: data only, 1 to 500 characters.
    #[schemars(length(min = 1, max = 500))]
    pub reason: String,
}

impl Entry {
    /// Eight hex digits of the entry's content: an id names an entry by its position and this,
    /// so an undo after the file changed removes that entry or nothing, never its neighbour.
    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let d = Sha256::digest(serde_json::to_vec(self).expect("an entry serialises"));
        d[..4].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// `c<position>-<fingerprint>`, the id of an entry in the API.
pub fn entry_id(position: usize, fingerprint: &str) -> String {
    format!("c{position}-{fingerprint}")
}

/// The position and fingerprint of an id from [`entry_id`].
pub fn parse_entry_id(id: &str) -> Option<(usize, &str)> {
    let (n, fp) = id.strip_prefix('c')?.split_once('-')?;
    Some((n.parse().ok()?, fp))
}

/// An end of a link, split: `service`, `service:path`, `service:path#symbol`.
pub fn split_end(end: &str) -> (&str, Option<&str>, Option<&str>) {
    let (head, symbol) = match end.split_once('#') {
        Some((h, s)) => (h, Some(s)),
        None => (end, None),
    };
    match head.split_once(':') {
        Some((s, p)) => (s, Some(p), symbol),
        None => (head, None, symbol),
    }
}

/// A file that didn't validate: where, what and the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// 1-based; 0 when the problem has no single place.
    pub line: usize,
    pub message: String,
    pub fix: String,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: ", self.line)?;
        }
        write!(f, "{}", self.message)
    }
}

/// A parsed file and the 1-based line each entry starts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub overrides: Overrides,
    pub lines: Vec<u32>,
}

/// The JSON Schema of [`Overrides`].
pub fn schema_json() -> String {
    let mut s = serde_json::to_string_pretty(&schemars::schema_for!(Overrides))
        .expect("a schema serialises");
    s.push('\n');
    s
}

fn invalid(line: usize, message: impl Into<String>, fix: &str) -> Invalid {
    Invalid {
        line,
        message: message.into(),
        fix: fix.into(),
    }
}

/// An end must name a service, and a path that stays inside it: no `..`, no leading `/`, no
/// control characters. Whether it names anything is for the index to say.
fn end_ok(end: &str) -> bool {
    let (service, path, symbol) = split_end(end);
    !service.is_empty()
        && !end.chars().any(char::is_control)
        && path.is_none_or(|p| {
            !p.is_empty() && !p.starts_with('/') && !p.split('/').any(|c| c == ".." || c.is_empty())
        })
        && symbol.is_none_or(|s| !s.is_empty() && path.is_some())
}

fn check(i: usize, e: &Entry, line: usize) -> Result<(), Invalid> {
    let bad = |m: String| {
        Err(invalid(
            line,
            format!("correction {}: {m}", i + 1),
            "compare the entry with cartograph.overrides.schema.json",
        ))
    };
    let k = &e.key;
    for (name, end) in [
        ("key.from", Some(&k.from)),
        ("key.to", Some(&k.to)),
        ("to", e.to.as_ref()),
    ] {
        if let Some(end) = end
            && !end_ok(end)
        {
            return bad(format!(
                "{name} isn't `service`, `service:path` or `service:path#symbol`"
            ));
        }
    }
    let want = match k.kind {
        Kind::Http => ("op", k.op.is_some(), k.topic.is_none() && k.table.is_none()),
        Kind::Event => (
            "topic",
            k.topic.is_some(),
            k.op.is_none() && k.table.is_none(),
        ),
        Kind::Db => (
            "table",
            k.table.is_some(),
            k.op.is_none() && k.topic.is_none(),
        ),
        Kind::Imports | Kind::Calls => ("", true, k.carried().is_none()),
    };
    // An added link joins two files, so it needs no op: the key names only what it can.
    if !(want.1 || e.action == Action::Add) || !want.2 {
        return bad(match want.0 {
            "" => "an imports or calls key carries no op, topic or table".into(),
            w => format!(
                "a {} key carries `{w}` and nothing else of op, topic, table",
                k.kind.as_str()
            ),
        });
    }
    if k.carried()
        .is_some_and(|c| c.is_empty() || c.chars().any(char::is_control))
    {
        return bad("op, topic and table are one line of text".into());
    }
    let (needs_to, needs_conf, no_conf) = match e.action {
        Action::Remove => (false, false, true),
        Action::Retarget => (true, false, true),
        Action::Set => (false, true, false),
        Action::Add => (false, false, false),
    };
    if needs_to != e.to.is_some() {
        return bad(if needs_to {
            "retarget needs `to`"
        } else {
            "only retarget takes `to`"
        }
        .into());
    }
    if (needs_conf && e.confidence.is_none()) || (no_conf && e.confidence.is_some()) {
        return bad(format!(
            "{} {} `confidence`",
            e.action.as_str(),
            if needs_conf { "needs" } else { "takes no" }
        ));
    }
    if e.action == Action::Add && [&k.from, &k.to].iter().any(|x| split_end(x).1.is_none()) {
        return bad("add links two files: both ends need a path".into());
    }
    if e.to.as_deref().is_some_and(|t| split_end(t).1.is_none()) {
        return bad("retarget moves a link to a file: `to` needs a path".into());
    }
    let n = e.reason.chars().count();
    if e.reason.trim().is_empty() || n > MAX_REASON {
        return bad(format!(
            "reason is {n} characters; it takes 1 to {MAX_REASON}"
        ));
    }
    Ok(())
}

/// The line of each entry: the `-` lines under the top-level `corrections:` key. A file written
/// some other way (flow style) puts every entry on the key's line.
fn entry_lines(text: &str, n: usize) -> Vec<u32> {
    let mut lines = text.lines().enumerate();
    let Some((key, _)) = lines.find(|(_, l)| l.starts_with("corrections:")) else {
        return vec![1; n];
    };
    let mut dashes = Vec::new();
    let mut indent = None;
    for (i, l) in lines {
        let t = l.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let at = l.len() - t.len();
        if at == 0 && !t.starts_with('-') {
            break;
        }
        if (t.starts_with("- ") || t == "-") && *indent.get_or_insert(at) == at {
            dashes.push(i as u32 + 1);
        }
    }
    if dashes.len() == n {
        dashes
    } else {
        vec![key as u32 + 1; n]
    }
}

/// Parses and validates the text of an overrides file.
pub fn parse(text: &str) -> Result<Parsed, Invalid> {
    if text.len() > MAX_FILE {
        return Err(invalid(
            0,
            format!("the file is {} bytes; the limit is 4 MB", text.len()),
            "remove corrections that no longer apply",
        ));
    }
    let located = |e: serde_norway::Error| {
        let line = e.location().map_or(0, |l| l.line());
        invalid(line, e.to_string(), "fix the YAML at that place")
    };
    let value: serde_json::Value = serde_norway::from_str(text).map_err(located)?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {}
        Some(v) => {
            return Err(invalid(
                0,
                format!("the file is version {v}; this cartograph reads version 1"),
                "upgrade cartograph to a release that reads this version",
            ));
        }
        None => {
            return Err(invalid(
                0,
                "the file has no `version: 1`",
                "add `version: 1` as the first key",
            ));
        }
    }
    let overrides: Overrides = serde_norway::from_str(text).map_err(located)?;
    if overrides.corrections.len() > MAX_ENTRIES {
        return Err(invalid(
            0,
            format!(
                "{} corrections; the limit is {MAX_ENTRIES}",
                overrides.corrections.len()
            ),
            "remove corrections that no longer apply",
        ));
    }
    let lines = entry_lines(text, overrides.corrections.len());
    for (i, e) in overrides.corrections.iter().enumerate() {
        check(i, e, lines[i] as usize)?;
    }
    Ok(Parsed { overrides, lines })
}

/// The text of the file and what it parsed to.
pub type FileRead = (String, Result<Parsed, Invalid>);

/// Reads the file as untrusted: a regular file (not a link, a device or a pipe), never more than
/// the limit. `None` when there is none.
pub fn read(path: &Path) -> Result<Option<FileRead>, std::io::Error> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() > MAX_FILE as u64 {
        let msg = "it isn't a regular file of at most 4 MB";
        return Ok(Some((
            String::new(),
            Err(invalid(0, msg, "make it a plain file")),
        )));
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_FILE as u64 + 1)
        .read_to_string(&mut text)?;
    let parsed = parse(&text);
    Ok(Some((text, parsed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"version: 1
stale: warn
corrections:
  # a note
  - action: remove
    key: {kind: http, from: web:src/api.ts, to: orders:src/orders.ts, op: "GET /orders/{id}"}
    reason: not a call, a log line
  - action: retarget
    key:
      kind: http
      from: web
      to: orders
      op: GET /x
    to: billing:src/x.ts#handler
    reason: moved
"#;

    #[test]
    fn reads_entries_with_their_lines() {
        let p = parse(GOOD).unwrap();
        assert_eq!(p.overrides.stale, Stale::Warn);
        assert_eq!(p.lines, [5, 8]);
        assert_eq!(p.overrides.corrections[1].action, Action::Retarget);
        assert_eq!(split_end("a:b/c.ts#f"), ("a", Some("b/c.ts"), Some("f")));
    }

    #[test]
    fn refuses_what_isnt_a_correction() {
        for (text, why) in [
            ("version: 2\n", "version 2"),
            ("corrections: []\n", "no `version"),
            (
                "version: 1\ncorrections:\n  - action: set\n    key: {kind: http, from: a, to: b, op: GET /}\n    reason: r\n",
                "needs `confidence`",
            ),
            (
                "version: 1\ncorrections:\n  - action: remove\n    key: {kind: http, from: a, to: b}\n    reason: r\n",
                "`op`",
            ),
            (
                "version: 1\ncorrections:\n  - action: remove\n    key: {kind: http, from: 'a:../x', to: b, op: GET /}\n    reason: r\n",
                "key.from",
            ),
            (
                "version: 1\ncorrections:\n  - action: add\n    key: {kind: http, from: a, to: b, op: GET /}\n    reason: r\n",
                "both ends",
            ),
            (
                "version: 1\ncorrections:\n  - action: remove\n    key: {kind: http, from: a, to: b, op: GET /}\n    reason: ''\n",
                "reason",
            ),
            (
                "version: 1\ncorrections:\n  - action: remove\n    why: x\n",
                "unknown field",
            ),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.to_string().contains(why), "{text}: {e}");
        }
        let long = format!(
            "version: 1\ncorrections:\n  - action: remove\n    key: {{kind: http, from: a, to: b, op: GET /}}\n    reason: {}\n",
            "x".repeat(501)
        );
        assert!(parse(&long).unwrap_err().to_string().contains("501"));
    }

    #[test]
    fn at_most_ten_thousand_entries() {
        let one =
            "  - action: remove\n    key: {kind: http, from: a, to: b, op: GET /}\n    reason: r\n";
        let text = format!("version: 1\ncorrections:\n{}", one.repeat(MAX_ENTRIES + 1));
        assert!(parse(&text).unwrap_err().to_string().contains("limit"));
        let text = format!("version: 1\ncorrections:\n{}", one.repeat(MAX_ENTRIES));
        assert_eq!(parse(&text).unwrap().lines.len(), MAX_ENTRIES);
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
    fn a_good_file_validates_in_a_third_party_validator() {
        let schema: serde_json::Value = serde_json::from_str(&schema_json()).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let as_json: serde_json::Value = serde_norway::from_str(GOOD).unwrap();
        assert!(validator.is_valid(&as_json));
        assert!(
            !validator
                .is_valid(&serde_json::json!({"version": 1, "corrections": [{"action": "cut"}]}))
        );
    }

    proptest::proptest! {
        // ponytail: a property test stands in for the YAML fuzz target until cargo-fuzz (HD-06).
        #[test]
        fn the_reader_never_panics(s in "\\PC{0,300}") {
            let _ = parse(&s);
        }
    }
}
