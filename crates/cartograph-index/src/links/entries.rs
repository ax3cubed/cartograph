//! Entrypoint facts of one blob that no query adapter makes (XS-20): a package's `bin`, its `main`,
//! `module` and `exports`, a Go `main` package, and the schedulers a source file registers
//! (node-cron, Nest's `@Cron`, python's `schedule` and APScheduler). Text in, facts out.
//!
//! Fact attributes: `kind` (`cli`, `export` or `job`), `name`, and for a package `target`, the
//! file the field names, relative to the package.json that names it.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use cartograph_facts::schema::{
    AttrValue, Attrs, BlobFact, Capped, Evidence, EvidenceType, FactKind, FactSpan,
};
use regex::Regex;

/// Facts kept per blob.
const MAX_FACTS: usize = 64;

/// Bump when what a blob's facts hold changes.
pub const VERSION: &str = "entries 1";

/// Whether a file of this path can hold such a fact.
pub fn applies(path: &str) -> bool {
    path.rsplit('/').next() == Some("package.json")
        || matches!(
            path.rsplit_once('.').map(|x| x.1),
            Some("go" | "py" | "ts" | "mts" | "cts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
        )
}

fn fact(line: u32, text: &str, attrs: &[(&str, &str)]) -> BlobFact {
    let attrs: BTreeMap<_, _> = attrs
        .iter()
        .map(|(k, v)| (Capped::truncated(k), AttrValue::Str(Capped::truncated(v))))
        .collect();
    let head: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let head: String = head.chars().take(200).collect();
    BlobFact {
        kind: FactKind::Entrypoint,
        span: FactSpan([line, 0, line, 0]),
        attrs: Attrs::new(attrs).expect("few attributes"),
        evidence: Evidence {
            kind: EvidenceType::Literal,
            text: Capped::truncated(&cartograph_core::secrets::mask(&head)),
        },
    }
}

fn has(src: &[u8], needle: &[u8]) -> bool {
    src.windows(needle.len()).any(|w| w == needle)
}

/// The facts of one file, sorted and without repeats.
pub fn scan(path: &str, src: &[u8]) -> Vec<BlobFact> {
    let mut out = Vec::new();
    match path.rsplit_once('.').map(|x| x.1) {
        Some("json") => package(src, &mut out),
        Some("go") => go_main(path, src, &mut out),
        _ => scheduled(src, &mut out),
    }
    out.sort();
    out.dedup();
    out.truncate(MAX_FACTS);
    out
}

/// The files a package.json exports: (subpath, target) pairs.
fn targets(v: &serde_json::Value, sub: &str, out: &mut Vec<(String, String)>) {
    use serde_json::Value::{Array, Object, String};
    match v {
        String(s) => out.push((sub.to_owned(), s.clone())),
        Array(a) => a.iter().for_each(|x| targets(x, sub, out)),
        Object(o) => {
            for (k, x) in o {
                targets(x, if k.starts_with('.') { k } else { sub }, out);
            }
        }
        _ => {}
    }
}

fn package(src: &[u8], out: &mut Vec<BlobFact>) {
    let Ok(serde_json::Value::Object(p)) = serde_json::from_slice(src) else {
        return;
    };
    let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let target = |t: &str| t.trim_start_matches("./").to_owned();
    match p.get("bin") {
        Some(serde_json::Value::String(t)) => {
            let n = name.rsplit('/').next().unwrap_or(name);
            out.push(fact(
                1,
                &format!("\"bin\": \"{t}\""),
                &[("kind", "cli"), ("name", n), ("target", &target(t))],
            ));
        }
        Some(serde_json::Value::Object(o)) => {
            for (n, t) in o {
                if let Some(t) = t.as_str() {
                    out.push(fact(
                        1,
                        &format!("\"bin\": {{\"{n}\": \"{t}\"}}"),
                        &[("kind", "cli"), ("name", n), ("target", &target(t))],
                    ));
                }
            }
        }
        _ => {}
    }
    let mut found = Vec::new();
    for field in ["main", "module"] {
        if let Some(v) = p.get(field) {
            targets(v, ".", &mut found);
        }
    }
    if let Some(v) = p.get("exports") {
        targets(v, ".", &mut found);
    }
    for (sub, t) in found {
        // A target with a `*` is a pattern, not a file.
        if t.contains('*') || t.is_empty() {
            continue;
        }
        let n = if sub == "." {
            name.to_owned()
        } else {
            format!("{name}{}", sub.trim_start_matches('.'))
        };
        out.push(fact(
            1,
            &format!("\"exports\": \"{t}\""),
            &[("kind", "export"), ("name", &n), ("target", &target(&t))],
        ));
    }
}

fn go_main(path: &str, src: &[u8], out: &mut Vec<BlobFact>) {
    if !has(src, b"func main(") {
        return;
    }
    let Ok(text) = std::str::from_utf8(src) else {
        return;
    };
    let is_main = text
        .lines()
        .take_while(|l| !l.starts_with("import") && !l.starts_with("func"))
        .any(|l| l.trim_end() == "package main");
    if let (true, Some(line)) = (
        is_main,
        text.lines().position(|l| l.starts_with("func main(")),
    ) {
        let dir = path.rsplit('/').nth(1).unwrap_or("main");
        out.push(fact(
            line as u32 + 1,
            "func main()",
            &[("kind", "cli"), ("name", dir)],
        ));
    }
}

static SCHEDULER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"cron\.schedule\(\s*['"`]([^'"`]+)['"`]|@Cron\(\s*['"`]?([^'"`,)]+)|(schedule\.every\()|(\badd_job\()"#,
    )
    .expect("a fixed pattern")
});

fn scheduled(src: &[u8], out: &mut Vec<BlobFact>) {
    // Most files register none: a byte look before any regex.
    if !(has(src, b"cron")
        || has(src, b"Cron")
        || has(src, b"schedule.every")
        || has(src, b"add_job("))
    {
        return;
    }
    let Ok(text) = std::str::from_utf8(src) else {
        return;
    };
    for c in SCHEDULER.captures_iter(text) {
        let m = c.get(0).expect("the whole match");
        let name = c
            .get(1)
            .or_else(|| c.get(2))
            .map_or(m.as_str().trim_end_matches('('), |g| g.as_str().trim());
        let line = 1 + text.as_bytes()[..m.start()]
            .iter()
            .filter(|b| **b == b'\n')
            .count() as u32;
        out.push(fact(line, m.as_str(), &[("kind", "job"), ("name", name)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(f: &[BlobFact]) -> Vec<String> {
        f.iter()
            .map(|f| {
                let a = |k: &str| match f.attrs.get(k) {
                    Some(AttrValue::Str(s)) => s.as_str().to_owned(),
                    _ => String::new(),
                };
                format!("{} {} {}", a("kind"), a("name"), a("target"))
            })
            .collect()
    }

    #[test]
    fn a_package_says_its_bins_and_its_entry_files() {
        let mut f = names(&scan(
            "pkg/package.json",
            br#"{"name":"@acme/tool","bin":{"tool":"./bin/tool.js"},"main":"dist/index.js",
               "exports":{".":{"import":"./src/index.ts"},"./util":"./src/util.ts","./x/*":"./x/*.js"}}"#,
        ));
        f.sort();
        assert_eq!(
            f,
            [
                "cli tool bin/tool.js",
                "export @acme/tool dist/index.js",
                "export @acme/tool src/index.ts",
                "export @acme/tool/util src/util.ts",
            ]
        );
    }

    #[test]
    fn a_go_main_package_is_a_cli() {
        let f = scan(
            "cmd/worker/main.go",
            b"package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println() }\n",
        );
        assert_eq!(names(&f), ["cli worker "]);
        assert!(scan("lib/x.go", b"package lib\nfunc main() {}\n").is_empty());
    }

    #[test]
    fn schedulers_are_jobs() {
        let mut f = names(&scan(
            "src/jobs.ts",
            b"cron.schedule('0 2 * * *', run);\n@Cron('45 * * * * *')\nclean() {}\n",
        ));
        f.sort();
        assert_eq!(f, ["job 0 2 * * * ", "job 45 * * * * * "]);
        assert!(scan("src/a.ts", b"const x = 1;").is_empty());
    }
}
