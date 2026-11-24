//! Secret hygiene (XS-10): nothing that looks like a secret is stored in a text column of the index
//! or the fact cache. Two tools, both on the `regex` crate (no backtracking, so a hostile string
//! can't stall them):
//!
//! - [`classify_config_value`]: a config value is kept only when it is a locator (URL, `host[:port]`
//!   or a short identifier) under a key that doesn't smell of a credential. Anything else is
//!   [`ConfigValue::Opaque`], with no value.
//! - [`Scanner`]: gitleaks-style rules compiled into one `RegexSet`, plus a Shannon-entropy check on
//!   long tokens. [`mask`] replaces what they find, for evidence snippets, raw references and commit
//!   summaries.

use std::borrow::Cow;
use std::sync::OnceLock;

use regex::{Regex, RegexBuilder, RegexSet};

/// What a secret is replaced with.
pub const MASK: &str = "[REDACTED]";

/// A rule pattern is cut here, and compiled with this size limit (1 MiB).
pub const MAX_RULE_BYTES: usize = 1024;
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Tokens this long are checked for entropy; shorter ones never are (cheap, and short words
/// can't be told from random text).
const ENTROPY_MIN_LEN: usize = 20;
/// Bits per character above which a token with letters and digits reads as random.
const ENTROPY_BITS: f64 = 4.0;

/// Key fragments that mark a value as a credential, upper-cased.
const DENY_KEYS: [&str; 9] = [
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "KEY",
    "CREDENTIAL",
    "PRIVATE",
    "DSN",
    "AUTH",
];

/// (name, pattern), after gitleaks' rules. When a pattern has a capture group 1, only that part is
/// masked, so the text around it (a URL's scheme and host) survives.
const BUILTIN: &[(&str, &str)] = &[
    ("aws-access-key", r"\b(?:AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16}\b"),
    (
        "aws-secret-key",
        r#"(?i)aws[A-Za-z0-9_ ]{0,20}(?:secret|key)[A-Za-z0-9_ ]{0,20}['"=: ]{1,4}([A-Za-z0-9/+]{40})\b"#,
    ),
    ("github-token", r"\bgh[pousr]_[A-Za-z0-9]{36,255}\b"),
    ("github-pat", r"\bgithub_pat_[A-Za-z0-9_]{22,255}\b"),
    ("gitlab-pat", r"\bglpat-[A-Za-z0-9_-]{20,}\b"),
    ("slack-token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}"),
    ("stripe-key", r"\b[sr]k_(?:live|test)_[0-9a-zA-Z]{16,}\b"),
    ("google-api-key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
    ("npm-token", r"\bnpm_[A-Za-z0-9]{36}\b"),
    (
        "jwt",
        r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    ),
    (
        "private-key",
        r"-----BEGIN [A-Z ]{0,20}PRIVATE KEY-----(?:[ -~\r\n]{0,3000}?-----END [A-Z ]{0,20}PRIVATE KEY-----)?",
    ),
    // `scheme://user:pw@host`: the userinfo goes, the rest stays.
    (
        "url-credentials",
        r#"\b[a-zA-Z][a-zA-Z0-9+.-]{1,20}://([^/\s:@'"]+:[^/\s@'"]*)@"#,
    ),
    // A quoted value only: `token: string` in a type is not a secret.
    (
        "generic-assignment",
        r#"(?i)\b(?:password|passwd|pwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token)\b['"]?\s*[:=]\s*['"]([^\s'"]{6,})['"]"#,
    ),
];

struct Rule {
    name: String,
    re: Regex,
}

/// The rules, matched in one pass.
pub struct Scanner {
    set: RegexSet,
    rules: Vec<Rule>,
}

/// One finding: the rule's name and the byte range to mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rule: String,
    pub start: usize,
    pub end: usize,
}

fn compile(pattern: &str) -> Result<Regex, String> {
    if pattern.len() > MAX_RULE_BYTES {
        return Err(format!("a rule is over {MAX_RULE_BYTES} bytes"));
    }
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| e.to_string())
}

impl Scanner {
    /// The built-in rules plus `custom` (name, pattern): size-limited and compile-checked, so a
    /// bad manifest rule is an error at load, not at the first scan.
    pub fn new(custom: &[(String, String)]) -> Result<Self, String> {
        let mut rules = Vec::new();
        for (name, pattern) in BUILTIN
            .iter()
            .map(|(n, p)| ((*n).to_owned(), (*p).to_owned()))
            .chain(custom.iter().cloned())
        {
            let re = compile(&pattern).map_err(|e| format!("secret rule {name}: {e}"))?;
            rules.push(Rule { name, re });
        }
        let set = RegexSet::new(rules.iter().map(|r| r.re.as_str())).map_err(|e| e.to_string())?;
        Ok(Self { set, rules })
    }

    /// The scanner with only the built-in rules.
    pub fn builtin() -> &'static Self {
        static S: OnceLock<Scanner> = OnceLock::new();
        S.get_or_init(|| Self::new(&[]).expect("built-in rules compile"))
    }

    /// Every finding in `text`, by position. One set pass decides which rules run again for their
    /// ranges.
    pub fn scan(&self, text: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for i in self.set.matches(text) {
            let rule = &self.rules[i];
            for c in rule.re.captures_iter(text) {
                let m = c.get(1).or_else(|| c.get(0)).expect("a match has group 0");
                hits.push(Hit {
                    rule: rule.name.clone(),
                    start: m.start(),
                    end: m.end(),
                });
            }
        }
        hits.extend(entropy_hits(text));
        hits.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        hits
    }

    /// `text` with every finding replaced by [`MASK`]; borrowed when there is none.
    pub fn mask<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let hits = self.scan(text);
        if hits.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for h in hits {
            if h.end <= at {
                continue; // inside a finding already masked
            }
            if h.start >= at {
                out.push_str(&text[at..h.start]);
                out.push_str(MASK);
            }
            // An overlap that runs past the last mask extends it.
            at = h.end;
        }
        out.push_str(&text[at..]);
        Cow::Owned(out)
    }
}

/// [`Scanner::mask`] with the built-in rules: what every writer calls.
pub fn mask(text: &str) -> Cow<'_, str> {
    Scanner::builtin().mask(text)
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '=' | '_' | '-')
}

/// Shannon entropy in bits per character, over the ASCII characters.
pub fn entropy(token: &str) -> f64 {
    let mut counts = [0u32; 128];
    let mut n = 0u32;
    for b in token.bytes().filter(u8::is_ascii) {
        counts[b as usize] += 1;
        n += 1;
    }
    let n = f64::from(n.max(1));
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / n;
            -p * p.log2()
        })
        .sum()
}

/// Whether one token reads as random: long, with letters and digits, and high entropy. Words,
/// identifiers and hex hashes stay under it.
fn looks_random(token: &str) -> bool {
    token.len() >= ENTROPY_MIN_LEN
        && token.bytes().any(|b| b.is_ascii_digit())
        && token.bytes().any(|b| b.is_ascii_alphabetic())
        && entropy(token) >= ENTROPY_BITS
}

/// Tokens of 20+ characters only: the check never looks at a shorter run.
fn entropy_hits(text: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (is_token_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                start = None;
                if i - s >= ENTROPY_MIN_LEN && looks_random(&text[s..i]) {
                    hits.push(Hit {
                        rule: "high-entropy".into(),
                        start: s,
                        end: i,
                    });
                }
            }
            _ => {}
        }
    }
    hits
}

/// What a config value is stored as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    /// A URL (without userinfo, query or fragment), `host[:port]` or a short identifier.
    Locator(String),
    /// Anything else: `value_kind = opaque`, no value kept.
    Opaque,
}

/// Longest short identifier (a topic, queue or table name) kept.
const MAX_IDENT: usize = 64;

fn denied_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    DENY_KEYS.iter().any(|d| k.contains(d))
}

/// `scheme://rest` without userinfo, query or fragment; `None` when it isn't a URL.
pub fn scrub_url(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if !scheme_ok {
        return None;
    }
    let rest = rest.split(['#', '?']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    // The last `@` ends the userinfo; a password can hold an `@`.
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(format!("{scheme}://{host}{path}"))
}

fn is_host_port(v: &str) -> bool {
    let (host, port) = match v.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (v, None),
    };
    let port_ok =
        port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    !host.is_empty()
        && port_ok
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

fn is_identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_IDENT
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':' | b'/'))
}

/// Classifies one config value under `key` (XS-10 (1)).
pub fn classify_config_value(key: &str, value: &str) -> ConfigValue {
    if denied_key(key) {
        return ConfigValue::Opaque;
    }
    let value = value.trim();
    let kept = if let Some(url) = scrub_url(value) {
        url
    } else if is_host_port(value) || is_identifier(value) {
        value.to_owned()
    } else {
        return ConfigValue::Opaque;
    };
    // A URL path or an identifier can still be a token pasted in the wrong place.
    if Scanner::builtin().scan(&kept).is_empty() {
        ConfigValue::Locator(kept)
    } else {
        ConfigValue::Opaque
    }
}

#[cfg(test)]
mod tests;
