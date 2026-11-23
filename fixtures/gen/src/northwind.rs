//! `northwind`: the demo workspace of `lib/workspace.ts` as a real repository, so the demo and the
//! goldens are one thing. The generator reads that file, so a change to the demo data is a change to
//! the fixture and shows up as a golden diff.
//!
//! Each service is a top-level folder named as in the demo (`api-gateway/src/routes/orders.ts` is
//! the demo node path). Commits carry the demo's dates (noon UTC), authors and messages, in order.
//! A file appears in the first commit that names it in a link, or in its service's first commit.
//! A TypeScript file's `imports`, `calls` and `event` links to TypeScript files are real imports
//! (relative inside a service, `@northwind/<service>/…` across services); every other link is a
//! comment, so it still changes the file's blob in the commit that adds or removes it.
//!
//! ponytail: one repository with six service folders, not six repositories, until indexing spans
//! repos (IDX-11); the demo's `repo` field maps to the top-level folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cartograph_git::fixture::Fixture;
use regex::Regex;

use crate::relative;

pub(crate) const SOURCE: &str = include_str!("northwind.rs");
pub(crate) const WORKSPACE_TS: &str = include_str!("../../../lib/workspace.ts");

pub struct Service {
    pub id: String,
    pub name: String,
    pub lang: String,
}

pub struct Commit {
    pub sha: String,
    pub date: String,
    pub author: String,
    pub repo: String,
    pub message: String,
}

pub struct Link {
    /// Demo node ids, `<service id>/<module>/<file>`.
    pub from: String,
    pub to: String,
    pub kind: String,
    pub added: String,
    pub removed: Option<String>,
}

/// The demo data, read from `lib/workspace.ts`.
pub struct Demo {
    pub services: Vec<Service>,
    /// (service id, module, file), in demo order.
    pub files: Vec<(String, String, String)>,
    pub commits: Vec<Commit>,
    pub links: Vec<Link>,
}

/// Parses the demo's tables. Panics, naming the table, if the file's shape changed.
pub fn demo() -> Demo {
    let ts = WORKSPACE_TS;
    let block = |start: &str| {
        let i = ts
            .find(start)
            .unwrap_or_else(|| panic!("{start} in lib/workspace.ts"));
        let end = ts[i..].find("\n];").expect("the table ends") + i;
        &ts[i..end]
    };
    let services: Vec<Service> =
        Regex::new(r#"\{ id: "(\w+)", name: "([\w-]+)", color: "[^"]*", lang: "(\w+)" \}"#)
            .unwrap()
            .captures_iter(block("export const SERVICES"))
            .map(|c| Service {
                id: c[1].into(),
                name: c[2].into(),
                lang: c[3].into(),
            })
            .collect();
    let module = Regex::new(r#"^\s*\["(\w+)", "(\w+)", "[^"]*", \[$"#).unwrap();
    let file = Regex::new(r#"^\s*\["([\w.]+)", "[^"]*"\],?$"#).unwrap();
    let mut files = Vec::new();
    let mut current = None;
    for line in block("const TREE").lines() {
        if let Some(c) = module.captures(line) {
            current = Some((c[1].to_owned(), c[2].to_owned()));
        } else if let (Some(c), Some((s, m))) = (file.captures(line), &current) {
            files.push((s.clone(), m.clone(), c[1].to_owned()));
        }
    }
    let commits: Vec<Commit> = Regex::new(
        r#"\{ sha: "(\w+)", date: "([\d-]+)", author: "([^"]+)", repo: "(\w+)", message: "([^"]+)" \}"#,
    )
    .unwrap()
    .captures_iter(block("export const COMMITS"))
    .map(|c| Commit {
        sha: c[1].into(),
        date: c[2].into(),
        author: c[3].into(),
        repo: c[4].into(),
        message: c[5].into(),
    })
    .collect();
    let short: BTreeMap<String, String> = Regex::new(r#"(\w) = "(\w+)""#)
        .unwrap()
        .captures_iter(
            ts.lines()
                .find(|l| l.starts_with("const g = "))
                .expect("the service shorthands"),
        )
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect();
    let links: Vec<Link> = Regex::new(
        r#"L\(`\$\{(\w)\}/([^`]+)`, `\$\{(\w)\}/([^`]+)`, "(\w+)", "(\w+)"(?:, "(\w+)")?\)"#,
    )
    .unwrap()
    .captures_iter(block("export const LINKS"))
    .map(|c| Link {
        from: format!("{}/{}", short[&c[1]], &c[2]),
        to: format!("{}/{}", short[&c[3]], &c[4]),
        kind: c[5].into(),
        added: c[6].into(),
        removed: c.get(7).map(|m| m.as_str().to_owned()),
    })
    .collect();
    assert!(
        !services.is_empty() && !files.is_empty() && !commits.is_empty() && !links.is_empty(),
        "lib/workspace.ts changed shape: services {}, files {}, commits {}, links {}",
        services.len(),
        files.len(),
        commits.len(),
        links.len()
    );
    Demo {
        services,
        files,
        commits,
        links,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Noon UTC of an ISO day, as the demo's `dayMs` places it, in seconds.
pub fn noon(iso: &str) -> i64 {
    let p: Vec<i64> = iso.split('-').map(|x| x.parse().expect("a date")).collect();
    days(p[0], p[1], p[2]) * 86_400 + 43_200
}

pub(crate) fn generate(root: &Path) {
    let d = demo();
    let svc = |id: &str| {
        d.services
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("service {id}"))
    };
    // Demo node id -> repository path.
    let path = |id: &str| {
        let (s, rest) = id.split_once('/').expect("a file id");
        let (m, f) = rest.split_once('/').expect("a file id");
        format!("{}/src/{m}/{f}", svc(s).name)
    };
    let ts = |id: &str| id.ends_with(".ts");
    // When each file first appears: its first link, else its service's first commit.
    let order: BTreeMap<&str, usize> = d
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.sha.as_str(), i))
        .collect();
    let mut born: BTreeMap<String, usize> = BTreeMap::new();
    for (s, m, f) in &d.files {
        let id = format!("{s}/{m}/{f}");
        let first_link = d
            .links
            .iter()
            .filter(|l| l.from == id || l.to == id)
            .map(|l| order[l.added.as_str()])
            .min();
        let first_commit = d.commits.iter().position(|c| &c.repo == s);
        born.insert(
            id,
            first_link
                .or(first_commit)
                .expect("a file has a link or a commit"),
        );
    }

    let mut f = Fixture::init(&root.join("northwind"));
    let mut written: BTreeSet<String> = BTreeSet::new();
    for (i, c) in d.commits.iter().enumerate() {
        // Manifests of TypeScript services, with the service's first file.
        for s in d.services.iter().filter(|s| s.lang == "TS") {
            let first = born
                .iter()
                .filter(|(id, _)| id.starts_with(&format!("{}/", s.id)))
                .map(|(_, b)| *b)
                .min();
            if first == Some(i) {
                f.write(
                    &format!("{}/package.json", s.name),
                    &format!(
                        "{{\n  \"name\": \"@northwind/{}\",\n  \"private\": true,\n  \"exports\": {{ \"./*\": \"./src/*.ts\" }}\n}}\n",
                        s.name
                    ),
                );
            }
        }
        if i == 0 {
            f.write(
                "package.json",
                "{\n  \"name\": \"northwind\",\n  \"private\": true\n}\n",
            );
            f.write(
                "pnpm-workspace.yaml",
                &format!(
                    "packages:\n{}",
                    d.services
                        .iter()
                        .filter(|s| s.lang == "TS")
                        .map(|s| format!("  - {}\n", s.name))
                        .collect::<String>()
                ),
            );
        }
        // Every file that exists by now, rewritten from the links live after this commit; unchanged
        // content leaves its blob alone.
        for (id, _) in born.iter().filter(|(_, b)| **b <= i) {
            let live: Vec<&Link> = d
                .links
                .iter()
                .filter(|l| {
                    &l.from == id
                        && order[l.added.as_str()] <= i
                        && l.removed.as_ref().is_none_or(|r| order[r.as_str()] > i)
                })
                .collect();
            let (s, _) = id.split_once('/').expect("a file id");
            let mut body = String::new();
            for l in &live {
                let (ts_s, _) = l.to.split_once('/').expect("a file id");
                let import =
                    ts(id) && ts(&l.to) && matches!(l.kind.as_str(), "imports" | "calls" | "event");
                if import {
                    let stem = l.to.trim_end_matches(".ts");
                    let spec = if ts_s == s {
                        relative(&path(id), &path(stem))
                    } else {
                        let (_, rest) = stem.split_once('/').expect("a file id");
                        format!("@northwind/{}/{rest}", svc(ts_s).name)
                    };
                    let name = stem.rsplit('/').next().expect("a stem");
                    body.push_str(&format!("import * as {name} from \"{spec}\";\n"));
                } else {
                    let c = if ts(id) { "//" } else { "#" };
                    body.push_str(&format!("{c} {} {}\n", l.kind, path(&l.to)));
                }
            }
            let stem = id.rsplit('/').next().expect("a name");
            let stem = stem.split('.').next().expect("a stem");
            body.push_str(&match id.rsplit('.').next() {
                Some("ts") => format!("export const {stem} = {};\n", live.len()),
                Some("go") => format!(
                    "package main\n\nfunc {stem}() int {{ return {} }}\n",
                    live.len()
                ),
                _ => format!("def {stem}():\n    return {}\n", live.len()),
            });
            f.write(&path(id), &body);
            written.insert(path(id));
        }
        let (name, email) = (c.author.clone(), author_email(&c.author));
        f.author = (name, email);
        f.set_clock(noon(&c.date) - 60);
        f.commit(&c.message);
    }
    assert_eq!(written.len(), d.files.len(), "every demo file is written");
}

fn author_email(name: &str) -> String {
    format!(
        "{}@northwind.example",
        name.to_lowercase().replace(' ', ".")
    )
}
