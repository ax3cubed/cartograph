//! Architecture rules and their baseline (XS-22, H6): the types of `rules:` in the manifest, their
//! validation, and `cartograph.baseline.json`. Evaluating a rule against the index is
//! `cartograph_index::rules`.
//!
//! Both files are committed by teammates, so both are untrusted (F8): counts are capped (1k rules,
//! 100k baseline entries), and a `path` is a regex compiled with the `regex` crate (no
//! backtracking) under a size limit.

use std::io::Read;
use std::path::Path;

use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const BASELINE_FILE: &str = "cartograph.baseline.json";
pub const MAX_RULES: usize = 1_000;
pub const MAX_BASELINE: usize = 100_000;
const MAX_BASELINE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PATTERN: usize = 512;
const SIZE_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    /// No link from `from` to `to`.
    Forbid,
    /// No cycle among the services `from` selects, over links of `link` kinds.
    NoCycles,
}

impl RuleType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forbid => "forbid",
            Self::NoCycles => "no_cycles",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Fails `cartograph check`.
    #[default]
    Error,
    Warn,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
        }
    }
}

/// A link kind of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Import,
    Package,
    Http,
    Event,
    Db,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Package => "package",
            Self::Http => "http",
            Self::Event => "event",
            Self::Db => "db",
        }
    }
}

/// The lowest level a link may have and still be a violation. A guessed link never is one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MinConfidence {
    #[default]
    Likely,
    Exact,
}

/// Which nodes an end of a rule means. Every field given must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Selector {
    /// Tags the service has all of: its `tags` in this file, plus `owner:<owner>` for each owner
    /// of one of its files (CODEOWNERS).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The service's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// A regex (unanchored) over a file's repo path, or a table's name. On `from`, groups may be
    /// captured; on `to`, `$1` to `$9` stand for what they captured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Selector {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    /// Names the rule in the baseline and in output: 1 to 100 characters, unique.
    pub name: String,
    #[serde(rename = "type")]
    pub kind: RuleType,
    #[serde(default, skip_serializing_if = "is_default")]
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Selector::is_empty")]
    pub from: Selector,
    #[serde(default, skip_serializing_if = "Selector::is_empty")]
    pub to: Selector,
    /// The link kinds the rule covers; any when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link: Vec<LinkKind>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub min_confidence: MinConfidence,
    /// Why, shown with a violation: data only, at most 500 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A path regex under the size limit.
pub fn compile(pattern: &str) -> Result<Regex, String> {
    if pattern.len() > MAX_PATTERN {
        return Err(format!("the pattern is over {MAX_PATTERN} bytes"));
    }
    RegexBuilder::new(pattern)
        .size_limit(SIZE_LIMIT)
        .dfa_size_limit(SIZE_LIMIT)
        .build()
        .map_err(|e| e.to_string())
}

fn placeholder() -> Regex {
    Regex::new(r"\$([1-9])").expect("a fixed pattern")
}

/// The highest `$N` a pattern uses.
pub fn refs(pattern: &str) -> usize {
    placeholder()
        .captures_iter(pattern)
        .filter_map(|c| c[1].parse().ok())
        .max()
        .unwrap_or(0)
}

/// What the groups of `caps` captured, group 1 first.
pub fn groups(caps: &regex::Captures<'_>) -> Vec<String> {
    (1..caps.len())
        .map(|i| caps.get(i).map_or("", |m| m.as_str()).to_owned())
        .collect()
}

/// `to` with each `$N` replaced by the escaped capture `N`.
pub fn fill(to: &str, groups: &[String]) -> String {
    placeholder()
        .replace_all(to, |c: &regex::Captures<'_>| {
            let n: usize = c[1].parse().unwrap_or(0);
            regex::escape(groups.get(n.wrapping_sub(1)).map_or("", String::as_str))
        })
        .into_owned()
}

/// Checks the rules: counts, names, regexes, and that every `$N` in `to` is a group of `from`.
/// The error is (what, fix).
pub fn validate(rules: &[Rule]) -> Result<(), (String, String)> {
    let bad = |what: String, fix: &str| Err((what, fix.to_owned()));
    if rules.len() > MAX_RULES {
        return bad(
            format!("{} rules; the limit is {MAX_RULES}", rules.len()),
            "remove rules",
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, r) in rules.iter().enumerate() {
        if !(1..=100).contains(&r.name.chars().count()) {
            return bad(
                format!("rules[{i}].name must be 1 to 100 characters"),
                "name the rule",
            );
        }
        if !seen.insert(r.name.as_str()) {
            return bad(
                format!("rules[{i}].name `{}` appears twice", r.name),
                "name each rule once",
            );
        }
        if r.reason.as_ref().is_some_and(|x| x.chars().count() > 500) {
            return bad(
                format!("rules[{i}].reason is over 500 characters"),
                "shorten it",
            );
        }
        let mut groups = 0;
        if let Some(p) = r.from.path.as_deref() {
            match compile(p) {
                Ok(re) => groups = re.captures_len() - 1,
                Err(e) => return bad(format!("rules[{i}].from.path: {e}"), "fix the regex"),
            }
        }
        if let Some(p) = r.to.path.as_deref() {
            if refs(p) > groups {
                return bad(
                    format!("rules[{i}].to.path uses a $N that from.path doesn't capture"),
                    "capture it with a group in from.path",
                );
            }
            // The same pattern with each `$N` as a stand-in word.
            if let Err(e) = compile(&placeholder().replace_all(p, "x")) {
                return bad(format!("rules[{i}].to.path: {e}"), "fix the regex");
            }
        }
        if r.kind == RuleType::NoCycles && (r.from.path.is_some() || r.to.path.is_some()) {
            return bad(
                format!("rules[{i}]: no_cycles works on services, so it takes no path"),
                "select services by tags or service",
            );
        }
    }
    Ok(())
}

/// A violation as the baseline keys it: names, not row ids, so it survives a rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Entry {
    pub rule: String,
    pub kind: String,
    /// `service:path`; a table's end is `service:table`.
    pub from: String,
    pub to: String,
    /// `GET /orders/{id}`, `read users`; empty when the link carries none.
    #[serde(default)]
    pub op: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    pub violations: Vec<Entry>,
}

impl Baseline {
    /// Sorted and deduplicated, so the file is the same whatever order the violations came in.
    pub fn new(mut violations: Vec<Entry>) -> Self {
        violations.sort();
        violations.dedup();
        Self {
            version: 1,
            violations,
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("a baseline serialises");
        s.push('\n');
        s
    }
}

/// Reads the baseline as untrusted: a regular file of at most 64 MB, version 1, at most
/// [`MAX_BASELINE`] entries. `None` when there is none.
pub fn read_baseline(path: &Path) -> Result<Option<Baseline>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_file() || meta.len() > MAX_BASELINE_BYTES {
        return Err("it isn't a regular file of at most 64 MB".into());
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_BASELINE_BYTES).read_to_string(&mut text))
        .map_err(|e| e.to_string())?;
    let b: Baseline = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if b.version != 1 {
        return Err(format!(
            "version {}; this cartograph reads version 1",
            b.version
        ));
    }
    if b.violations.len() > MAX_BASELINE {
        return Err(format!(
            "{} entries; the limit is {MAX_BASELINE}",
            b.violations.len()
        ));
    }
    Ok(Some(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, from: &str, to: &str) -> Rule {
        let sel = |p: &str| Selector {
            path: (!p.is_empty()).then(|| p.to_owned()),
            ..Selector::default()
        };
        Rule {
            name: name.into(),
            kind: RuleType::Forbid,
            severity: Severity::Error,
            from: sel(from),
            to: sel(to),
            link: vec![],
            min_confidence: MinConfidence::Likely,
            reason: None,
        }
    }

    #[test]
    fn captures_must_exist() {
        assert!(validate(&[rule("r", "^a/([^/]+)/", "^b/$1/")]).is_ok());
        assert!(validate(&[rule("r", "^a/", "^b/$1/")]).is_err());
        assert!(validate(&[rule("r", "(", "")]).is_err());
        assert!(validate(&[rule("r", &"a".repeat(600), "")]).is_err());
        // A pattern that would build a huge program is refused, not run.
        assert!(validate(&[rule("r", "((((a{100}){100}){100}){100})", "")]).is_err());
    }

    #[test]
    fn names_are_unique_and_counts_capped() {
        assert!(validate(&[rule("r", "", ""), rule("r", "", "")]).is_err());
        let many: Vec<Rule> = (0..=MAX_RULES)
            .map(|i| rule(&i.to_string(), "", ""))
            .collect();
        assert!(validate(&many).is_err());
    }

    #[test]
    fn a_baseline_is_sorted_and_round_trips() {
        let e = |r: &str| Entry {
            rule: r.into(),
            kind: "db".into(),
            from: "a:x".into(),
            to: "b:t".into(),
            op: "read t".into(),
        };
        let b = Baseline::new(vec![e("z"), e("a"), e("a")]);
        assert_eq!(b.violations.len(), 2);
        assert_eq!(b.violations[0].rule, "a");
        let back: Baseline = serde_json::from_str(&b.to_json()).unwrap();
        assert_eq!(back, b);
    }

    proptest::proptest! {
        // ponytail: a property test stands in for the selector fuzz target until cargo-fuzz (HD-06).
        #[test]
        fn validating_a_selector_never_panics(a in "\\PC{0,80}", b in "\\PC{0,80}") {
            let _ = validate(&[rule("r", &a, &b)]);
        }
    }

    #[test]
    fn a_capture_is_escaped_in_the_far_end() {
        let re = compile("^a/(.+)/x").unwrap();
        let caps = re.captures("a/b.c/x").unwrap();
        assert_eq!(fill("^b/$1/", &groups(&caps)), "^b/b\\.c/");
    }
}
