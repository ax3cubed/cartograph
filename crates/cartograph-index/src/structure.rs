//! Structure nodes (IDX-20): the services, libraries and modules the app's levels of detail show,
//! over the file lineages.
//!
//! Service detection (C6), per repository, first match wins:
//! 1. `services:` entries of the manifest for this repository, and each of its scopes that no entry
//!    covers, one service each (the manifest names both, so they come before anything guessed;
//!    proposed);
//! 2. Backstage `catalog-info.yaml` components;
//! 3. workspace manifests: pnpm, npm workspaces, go.work, Nx `project.json`, Cargo, Composer path
//!    repositories; a package with a deployable marker (Dockerfile, a compose build, a k8s manifest
//!    beside it, Procfile, serverless config) is a service, the rest are libraries;
//! 4. otherwise the repository is one service.
//!
//! Source files no root covers belong to a remainder service named after the repository; a
//! manifest no root covers (a package.json outside every scope, read for package keys) belongs to
//! none. Manifests are
//! read at the indexed commit only, as data with a size cap; nothing is run (C6). History follows
//! each lineage's path against the current roots (R9, proposed): a file's service is the one its
//! path is under, at every step.
//!
//! Modules are folders under a service root that directly hold source files (proposed). A
//! service's or module's validity is the union of its files' path intervals (`node_span`). Every
//! node gets `parent` (one level up) and `service` (its top-level ancestor).
//!
//! ponytail: a module whose files all moved doesn't follow them to the new folder (IDX-18's
//! proposed rule); the new folder is a new module.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cartograph_git::{ObjectId, Repo, read_blob};
use cartograph_parse::extractor_for;
use cartograph_store::rusqlite::{self, Connection, OptionalExtension, params};
use serde_json::Value;

use crate::timeline::{excluded, glob_matches};

/// Manifests read for detection are capped at this size (IDX-20 security).
const MAX_MANIFEST: u64 = 1_000_000;

/// One `services:` entry of the manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceSpec {
    pub name: String,
    pub repo: String,
    /// A folder in the repository; the whole repository when `None`.
    pub path: Option<String>,
}

/// A service or library root in one repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Root {
    /// Repository-relative, without a trailing slash; "" for the whole repository.
    pub path: String,
    pub name: String,
    /// `service` or `library`.
    pub kind: &'static str,
    /// The C6 rule that found it (board S04).
    pub detected_by: String,
}

/// What detection found in one repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detected {
    pub roots: Vec<Root>,
    /// Host names that reach a root: (host, root path, source file).
    pub aliases: Vec<(String, String, String)>,
}

fn base(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn dir(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join(d: &str, rest: &str) -> String {
    let mut parts: Vec<&str> = if d.is_empty() {
        Vec::new()
    } else {
        d.split('/').collect()
    };
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Whether `path` is `root` or below it.
fn under(root: &str, path: &str) -> bool {
    root.is_empty() || path == root || path.strip_prefix(root).is_some_and(|r| r.starts_with('/'))
}

fn interesting(p: &str) -> bool {
    let b = base(p);
    matches!(
        b,
        "catalog-info.yaml"
            | "catalog-info.yml"
            | "pnpm-workspace.yaml"
            | "package.json"
            | "go.work"
            | "go.mod"
            | "project.json"
            | "Cargo.toml"
            | "composer.json"
            | "Procfile"
            | "serverless.yml"
            | "serverless.yaml"
            | "serverless.ts"
            | "serverless.js"
    ) || b == "Dockerfile"
        || b.starts_with("Dockerfile.")
        || b.ends_with(".Dockerfile")
        || ((b.starts_with("docker-compose") || b.starts_with("compose"))
            && (b.ends_with(".yml") || b.ends_with(".yaml")))
        || (is_k8s_dir(p) && (b.ends_with(".yml") || b.ends_with(".yaml")))
}

/// k8s manifests are read only in folders named for them, so a repository's other YAML is never
/// parsed.
fn is_k8s_dir(p: &str) -> bool {
    dir(p)
        .split('/')
        .any(|d| matches!(d, "k8s" | "kubernetes" | "kube" | "deploy" | "manifests"))
}

/// Finds the service roots of one repository at `tip` (C6).
pub fn detect(
    repo: &Repo,
    tip: ObjectId,
    name: &str,
    scopes: &[String],
    services: &[ServiceSpec],
    exclude: &[String],
) -> Result<Detected, cartograph_git::GitError> {
    let mine: Vec<Root> = services
        .iter()
        .filter(|s| s.repo == name)
        .map(|s| Root {
            path: s.path.as_deref().map_or(String::new(), |p| join("", p)),
            name: s.name.clone(),
            kind: "service",
            detected_by: "manifest".into(),
        })
        .collect();
    let skipped = |p: &str| {
        p.split('/')
            .rev()
            .skip(1)
            .any(|d| matches!(d, "node_modules" | "dist" | "build" | "vendor" | ".next"))
    };
    let tree = repo.tree_at(tip, |p| {
        interesting(p) && !skipped(p) && !excluded(exclude, p)
    })?;
    let local = repo.local();
    let text = |p: &str| -> Option<String> {
        let oid = tree.get(p)?;
        String::from_utf8(read_blob(&local, *oid, MAX_MANIFEST).ok()?).ok()
    };
    let json = |p: &str| -> Option<Value> { serde_json::from_str(&text(p)?).ok() };
    let yaml_docs = |p: &str| -> Vec<Value> {
        use serde::Deserialize;
        let Some(t) = text(p) else { return Vec::new() };
        serde_norway::Deserializer::from_str(&t)
            .take(100)
            .filter_map(|d| Value::deserialize(d).ok())
            .collect()
    };

    // Compose services by build context, and k8s names by folder: aliases, and deployable markers.
    let mut aliases_at: Vec<(String, String, String)> = Vec::new();
    for p in tree.keys() {
        let b = base(p);
        if (b.starts_with("docker-compose") || b.starts_with("compose")) && !is_k8s_dir(p) {
            for doc in yaml_docs(p) {
                let Some(svcs) = doc.get("services").and_then(Value::as_object) else {
                    continue;
                };
                for (host, def) in svcs {
                    let ctx = match def.get("build") {
                        Some(Value::String(s)) => Some(s.as_str()),
                        Some(b) => b.get("context").and_then(Value::as_str),
                        None => None,
                    };
                    if let Some(ctx) = ctx {
                        aliases_at.push((host.clone(), join(dir(p), ctx), p.clone()));
                    }
                }
            }
        } else if is_k8s_dir(p) {
            for doc in yaml_docs(p) {
                let kind = doc.get("kind").and_then(Value::as_str).unwrap_or("");
                if matches!(kind, "Deployment" | "Service" | "StatefulSet")
                    && let Some(n) = doc.pointer("/metadata/name").and_then(Value::as_str)
                {
                    aliases_at.push((n.to_owned(), dir(p).to_owned(), p.clone()));
                }
            }
        }
    }
    let deployable = |d: &str| {
        tree.keys().any(|p| {
            let b = base(p);
            dir(p) == d
                && (b == "Dockerfile"
                    || b.starts_with("Dockerfile.")
                    || b.ends_with(".Dockerfile")
                    || b == "Procfile"
                    || b.starts_with("serverless."))
        }) || aliases_at
            .iter()
            .any(|(_, at, src)| at == d && !is_k8s_dir(src))
            || aliases_at
                .iter()
                .any(|(_, at, src)| is_k8s_dir(src) && under(d, at) && !d.is_empty())
    };

    let mut mine = mine;
    for s in scopes {
        let path = join("", s);
        if !mine.iter().any(|r| under(&r.path, &path)) {
            mine.push(Root {
                name: base(&path).to_owned(),
                path,
                kind: "service",
                detected_by: "scope".into(),
            });
        }
    }
    let roots = if !mine.is_empty() {
        mine
    } else {
        let mut found: Vec<Root> = Vec::new();
        for p in tree.keys().filter(|p| base(p).starts_with("catalog-info.")) {
            for doc in yaml_docs(p) {
                if doc.get("kind").and_then(Value::as_str) == Some("Component")
                    && let Some(n) = doc.pointer("/metadata/name").and_then(Value::as_str)
                {
                    found.push(Root {
                        path: dir(p).to_owned(),
                        name: n.to_owned(),
                        kind: "service",
                        detected_by: "backstage".into(),
                    });
                }
            }
        }
        if found.is_empty() {
            found = workspace_packages(&tree, &json, &text);
            for r in &mut found {
                if !deployable(&r.path) {
                    r.kind = "library";
                }
            }
        }
        found
    };
    let mut roots: Vec<Root> = roots
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // One root per path: the first by name wins.
    roots.dedup_by(|b, a| a.path == b.path);
    if roots.is_empty() {
        roots.push(Root {
            path: String::new(),
            name: name.to_owned(),
            kind: "service",
            detected_by: "repo".into(),
        });
    }
    let aliases = aliases_at
        .into_iter()
        .filter_map(|(host, at, src)| {
            let r = roots
                .iter()
                .filter(|r| under(&r.path, &at))
                .max_by_key(|r| r.path.len())?;
            Some((host, r.path.clone(), src))
        })
        .collect();
    Ok(Detected { roots, aliases })
}

/// Rule 4: the packages a workspace manifest at the repository root lists.
fn workspace_packages(
    tree: &BTreeMap<String, ObjectId>,
    json: &dyn Fn(&str) -> Option<Value>,
    text: &dyn Fn(&str) -> Option<String>,
) -> Vec<Root> {
    let globs = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|g| !g.starts_with('!'))
                    .map(|g| join("", g))
                    .collect()
            })
            .unwrap_or_default()
    };
    // Folders holding `marker` that a glob matches, named by `name_of` or their folder name.
    let packages = |patterns: &[String], marker: &str, by: &str| -> Vec<Root> {
        tree.keys()
            .filter(|p| base(p) == marker && !dir(p).is_empty())
            .filter(|p| patterns.iter().any(|g| glob_matches(g, dir(p))))
            .map(|p| {
                let name = match marker {
                    "package.json" | "composer.json" | "project.json" => json(p)
                        .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned)),
                    "Cargo.toml" => text(p).and_then(|t| {
                        t.parse::<toml::Table>().ok().and_then(|t| {
                            t.get("package")?.get("name")?.as_str().map(str::to_owned)
                        })
                    }),
                    _ => None,
                };
                Root {
                    path: dir(p).to_owned(),
                    name: name.unwrap_or_else(|| base(dir(p)).to_owned()),
                    kind: "service",
                    detected_by: by.into(),
                }
            })
            .collect()
    };
    if let Some(t) = text("pnpm-workspace.yaml") {
        let v: Value = serde_norway::from_str(&t).unwrap_or(Value::Null);
        let found = packages(&globs(v.get("packages")), "package.json", "pnpm");
        if !found.is_empty() {
            return found;
        }
    }
    if let Some(v) = json("package.json") {
        let w = v.get("workspaces");
        let list = globs(w.and_then(|w| w.get("packages")).or(w));
        let found = packages(&list, "package.json", "npm-workspaces");
        if !found.is_empty() {
            return found;
        }
    }
    if let Some(t) = text("go.work") {
        let uses = go_work_uses(&t);
        let found = packages(&uses, "go.mod", "go.work");
        if !found.is_empty() {
            return found;
        }
    }
    let nx = packages(&["**".into()], "project.json", "nx");
    if !nx.is_empty() {
        return nx;
    }
    if let Some(t) = text("Cargo.toml")
        && let Ok(v) = t.parse::<toml::Table>()
    {
        let members: Vec<String> = v
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|g| join("", g))
                    .collect()
            })
            .unwrap_or_default();
        let found = packages(&members, "Cargo.toml", "cargo");
        if !found.is_empty() {
            return found;
        }
    }
    if let Some(v) = json("composer.json") {
        let urls: Vec<String> = v
            .get("repositories")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|r| r.get("type").and_then(Value::as_str) == Some("path"))
                    .filter_map(|r| r.get("url").and_then(Value::as_str))
                    .map(|g| join("", g))
                    .collect()
            })
            .unwrap_or_default();
        let found = packages(&urls, "composer.json", "composer");
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The folders a go.work `use` directive names, one-line or block form.
pub(crate) fn go_work_uses(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block = false;
    for line in t.lines() {
        let l = line.split("//").next().unwrap_or("").trim();
        if block {
            if l.starts_with(')') {
                block = false;
            } else if !l.is_empty() {
                out.push(join("", l.trim_matches('"')));
            }
        } else if let Some(rest) = l.strip_prefix("use") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                block = true;
            } else if !rest.is_empty() {
                out.push(join("", rest.trim_matches('"')));
            }
        }
    }
    out
}

/// A source file: one an extractor parses for imports, not a manifest.
fn is_source(path: &str) -> bool {
    extractor_for(path).is_some_and(|(_, k)| !k.starts_with("manifest-"))
}

/// An interval of steps; `None` is open.
type Span = (i64, Option<i64>);

/// Merges intervals; `None` is open.
fn merge(mut v: Vec<(i64, Option<i64>)>) -> Vec<(i64, Option<i64>)> {
    v.sort_unstable_by_key(|(f, _)| *f);
    let mut out: Vec<(i64, Option<i64>)> = Vec::new();
    for (f, t) in v {
        match out.last_mut() {
            Some((_, end)) if end.is_none_or(|e| f <= e) => {
                *end = match (*end, t) {
                    (None, _) | (_, None) => None,
                    (Some(a), Some(b)) => Some(a.max(b)),
                };
            }
            _ => out.push((f, t)),
        }
    }
    out
}

fn upsert(
    conn: &Connection,
    key: &str,
    kind: &str,
    name: &str,
    detected_by: Option<&str>,
) -> rusqlite::Result<i64> {
    conn.prepare_cached(
        "INSERT INTO node (kind, key, name, detected_by) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (key) DO UPDATE SET kind = excluded.kind, name = excluded.name,
           detected_by = excluded.detected_by",
    )?
    .execute(params![kind, key, name, detected_by])?;
    conn.prepare_cached("SELECT id FROM node WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
}

/// What [`build`] wrote, for counts and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Built {
    pub services: usize,
    pub modules: usize,
    pub files: usize,
}

/// Writes one repository's structure from its roots: service and module nodes, their spans, every
/// node's parent and service, and, when given, its aliases (otherwise they stay as they are). `prefix` is the repository's path prefix in a
/// multi-repository index. Nodes are never deleted; a root no longer detected keeps its node with
/// no span and no `detected_by`.
pub fn build(
    conn: &Connection,
    repo_id: i64,
    repo_name: &str,
    prefix: Option<&str>,
    roots: &[Root],
    aliases: Option<&[(String, String, String)]>,
) -> rusqlite::Result<Built> {
    let ns = |p: &str| match (prefix, p.is_empty()) {
        (Some(x), true) => x.to_owned(),
        (Some(x), false) => format!("{x}/{p}"),
        (None, _) => p.to_owned(),
    };
    let rel = |p: &str| -> String {
        match prefix {
            Some(x) => p
                .strip_prefix(x)
                .map_or(p, |r| r.trim_start_matches('/'))
                .to_owned(),
            None => p.to_owned(),
        }
    };
    let mut roots = roots.to_vec();
    let rest = roots.len();
    let remainder = Root {
        path: String::new(),
        name: repo_name.to_owned(),
        kind: "service",
        detected_by: "repo".into(),
    };
    if !roots.iter().any(|r| r.path.is_empty()) {
        roots.push(remainder);
    }
    let root_of = |p: &str| -> usize {
        let mut best = None;
        for (i, r) in roots.iter().enumerate() {
            if under(&r.path, p) && best.is_none_or(|b: usize| roots[b].path.len() < r.path.len()) {
                best = Some(i);
            }
        }
        best.expect("the remainder covers every path")
    };

    // Every path row of the repository, oldest first per lineage.
    let mut st = conn.prepare(
        "SELECT fp.file, fp.path, fp.from_seq, fp.to_seq FROM file_path fp
         JOIN file f ON f.id = fp.file WHERE f.repo = ?1 ORDER BY fp.file, fp.from_seq",
    )?;
    let mut service_spans: BTreeMap<usize, Vec<(i64, Option<i64>)>> = BTreeMap::new();
    let mut module_spans: BTreeMap<String, (usize, Vec<Span>)> = BTreeMap::new();
    let mut last_path: BTreeMap<i64, String> = BTreeMap::new();
    let mut rows = st.query([repo_id])?;
    while let Some(r) = rows.next()? {
        let (file, path, from, to): (i64, String, i64, Option<i64>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let rp = rel(&path);
        let root = root_of(&rp);
        let source = is_source(&path);
        if root >= rest && !source {
            // A manifest outside every root: read for package keys, part of no service.
            continue;
        }
        service_spans.entry(root).or_default().push((from, to));
        if source {
            module_spans
                .entry(dir(&path).to_owned())
                .or_insert_with(|| (root, Vec::new()))
                .1
                .push((from, to));
        }
        last_path.insert(file, path);
    }
    drop(rows);

    // Service nodes: the used roots, and any root of this repository no longer found loses its span.
    let prefix_key = format!("service:{repo_name}:");
    conn.execute(
        "DELETE FROM node_span WHERE node IN (SELECT id FROM node WHERE key >= ?1 AND key < ?2
           OR service IN (SELECT id FROM node WHERE key >= ?1 AND key < ?2))",
        params![prefix_key, format!("service:{repo_name};")],
    )?;
    if aliases.is_some() {
        conn.execute(
            "DELETE FROM alias WHERE node IN (SELECT id FROM node WHERE key >= ?1 AND key < ?2)",
            params![prefix_key, format!("service:{repo_name};")],
        )?;
    }
    conn.execute(
        "UPDATE node SET detected_by = NULL WHERE key >= ?1 AND key < ?2",
        params![prefix_key, format!("service:{repo_name};")],
    )?;
    let mut service_id: HashMap<usize, i64> = HashMap::new();
    let span = |node: i64, spans: Vec<(i64, Option<i64>)>| -> rusqlite::Result<()> {
        for (f, t) in merge(spans) {
            conn.prepare_cached(
                "INSERT INTO node_span (node, from_seq, to_seq) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![node, f, t])?;
        }
        Ok(())
    };
    for (i, spans) in service_spans {
        let r = &roots[i];
        let id = upsert(
            conn,
            &format!("{prefix_key}{}", r.path),
            r.kind,
            &r.name,
            Some(&r.detected_by),
        )?;
        conn.prepare_cached("UPDATE node SET parent = NULL, service = ?1 WHERE id = ?1")?
            .execute([id])?;
        span(id, spans)?;
        service_id.insert(i, id);
    }
    let mut module_id: HashMap<String, i64> = HashMap::new();
    for (d_path, (root, spans)) in &module_spans {
        let rp = rel(d_path);
        let name = rp
            .strip_prefix(&roots[*root].path)
            .map(|r| r.trim_start_matches('/'))
            .filter(|r| !r.is_empty())
            .unwrap_or(".");
        let id = upsert(conn, &format!("module:{d_path}"), "module", name, None)?;
        let svc = service_id[root];
        conn.prepare_cached("UPDATE node SET parent = ?2, service = ?2 WHERE id = ?1")?
            .execute(params![id, svc])?;
        conn.prepare_cached("DELETE FROM node_span WHERE node = ?1")?
            .execute([id])?;
        span(id, spans.clone())?;
        module_id.insert(d_path.clone(), id);
    }
    // Each file under its last path's module, or its service for a manifest.
    let current: HashMap<i64, (i64, Option<i64>, Option<i64>)> = conn
        .prepare(
            "SELECT n.file, n.id, n.parent, n.service FROM node n JOIN file f ON f.id = n.file
             WHERE f.repo = ?1 AND n.kind = 'file'",
        )?
        .query_map([repo_id], |r| {
            Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
        })?
        .collect::<Result<_, _>>()?;
    let mut files = 0;
    for (file, path) in &last_path {
        let Some(&(node, parent, service)) = current.get(file) else {
            continue;
        };
        let Some(&svc) = service_id.get(&root_of(&rel(path))) else {
            continue;
        };
        files += 1;
        let want = module_id.get(dir(path)).copied().unwrap_or(svc);
        if parent != Some(want) || service != Some(svc) {
            conn.prepare_cached("UPDATE node SET parent = ?2, service = ?3 WHERE id = ?1")?
                .execute(params![node, want, svc])?;
        }
    }
    for (host, at, src) in aliases.unwrap_or_default() {
        let id: Option<i64> = conn
            .prepare_cached("SELECT id FROM node WHERE key = ?1")?
            .query_row([format!("{prefix_key}{at}")], |r| r.get(0))
            .optional()?;
        if let Some(id) = id {
            conn.prepare_cached("INSERT INTO alias (host, node, source) VALUES (?1, ?2, ?3)")?
                .execute(params![host, id, ns(src)])?;
        }
    }
    Ok(Built {
        services: service_id.len(),
        modules: module_id.len(),
        files,
    })
}

/// The workspace's (services, modules, files) live at the last step, in one query
/// ([`cartograph_store::summary_counts`]).
pub fn summary_counts(conn: &Connection) -> rusqlite::Result<(i64, i64, i64)> {
    cartograph_store::summary_counts(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_work_use_forms() {
        let t = "go 1.22\nuse ./api\nuse (\n  ./billing // the money\n  \"./shared\"\n)\n";
        assert_eq!(go_work_uses(t), ["api", "billing", "shared"]);
    }

    #[test]
    fn spans_merge() {
        assert_eq!(
            merge(vec![(5, Some(9)), (0, Some(3)), (3, Some(4)), (8, None)]),
            [(0, Some(4)), (5, None)]
        );
    }
}
