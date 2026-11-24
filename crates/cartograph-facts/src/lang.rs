//! Resolution for Go, Python and PHP (IDX-23), over the replay's tree as oxc_resolver's is for
//! TypeScript. Every path a resolution looks at is logged on the [`FsState`], so it becomes a key in
//! the reverse index; so does one synthetic key per language (`\0go`, `\0py`, `\0php`), which the
//! replay dirties whenever a manifest of that language appears, goes or changes, since a new
//! go.mod, pyproject.toml or composer.json can answer an import that waited on no path.
//!
//! - Go: the longest module path (go.mod `module`, or a `replace` to a folder) that prefixes the
//!   import names a folder; the link goes to that package folder (`dir/`), since Go imports name
//!   packages, not files. Not found: the standard library (no dot in the first segment) or an
//!   external module.
//! - Python: `a.b` is `a/b.py` or `a/b/__init__.py` under a package root: the folders of
//!   pyproject.toml and setup.cfg files with their declared roots and `src/`, the importer's top
//!   folder and the tree's root. `from a import b` tries the submodule `a/b` first. Relative imports
//!   start at the importer's folder. A top-level name that matches a pyproject's distribution name
//!   links to that pyproject, as a bare npm import links to a package.json.
//! - PHP: a `use`d class goes through composer.json `autoload` psr-4 (longest prefix first), then
//!   psr-0; a literal `require`/`include` is relative to the file's folder.

use std::collections::BTreeMap;

use cartograph_parse::Fact;

use crate::fs::FsState;
use crate::replay::{LinkKind, Outcome};

/// A manifest's fields and their values.
type Fields = Vec<(String, serde_json::Value)>;

/// The manifests of one language family, by path: what the resolvers read.
#[derive(Default, Clone)]
pub struct LangIndex {
    manifests: BTreeMap<String, (&'static str, Fields)>,
    derived: Option<Derived>,
}

#[derive(Default, Clone)]
struct Derived {
    /// Module path -> folder.
    go: Vec<(String, String)>,
    /// Package roots, sorted.
    py_roots: Vec<String>,
    /// Normalised distribution name -> pyproject path.
    py_dists: BTreeMap<String, String>,
    /// (namespace prefix, folder), longest prefix first.
    psr4: Vec<(String, String)>,
    psr0: Vec<(String, String)>,
}

/// The synthetic key of a manifest kind's language, if it has one.
pub fn lang_key(kind: &str) -> Option<&'static str> {
    if kind.starts_with("manifest-go") {
        Some("\0go")
    } else if kind.starts_with("manifest-py") {
        Some("\0py")
    } else if kind.starts_with("manifest-php") {
        Some("\0php")
    } else {
        None
    }
}

fn dir(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `d` joined with a relative path, `..` and `.` folded; `None` above the root.
pub fn join(d: &str, rest: &str) -> Option<String> {
    let mut parts: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
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

fn py_name(n: &str) -> String {
    n.to_lowercase().replace(['-', '.'], "_")
}

impl LangIndex {
    /// Records a manifest's facts (`None`: gone).
    pub fn update(&mut self, path: &str, kind: &'static str, facts: Option<&[Fact]>) {
        match facts {
            Some(f) => {
                let fields = f
                    .iter()
                    .filter_map(|x| match x {
                        Fact::Manifest { field, value } => {
                            Some((field.clone(), serde_json::from_str(value).ok()?))
                        }
                        _ => None,
                    })
                    .collect();
                self.manifests.insert(path.to_owned(), (kind, fields));
            }
            None => {
                self.manifests.remove(path);
            }
        }
        self.derived = None;
    }

    fn derived(&mut self) -> &Derived {
        if self.derived.is_none() {
            let mut d = Derived::default();
            for (path, (kind, fields)) in &self.manifests {
                let at = dir(path);
                let get = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v);
                match *kind {
                    "manifest-go/mod" => {
                        if let Some(m) = get("module").and_then(|v| v.as_str()) {
                            d.go.push((m.to_owned(), at.to_owned()));
                        }
                        for pair in get("replace")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                        {
                            if let (Some(old), Some(Some(new))) = (
                                pair.get(0).and_then(|v| v.as_str()),
                                pair.get(1).and_then(|v| v.as_str()).map(|n| join(at, n)),
                            ) {
                                d.go.push((old.to_owned(), new));
                            }
                        }
                    }
                    "manifest-py/pyproject" | "manifest-py/setup" => {
                        d.py_roots.push(at.to_owned());
                        d.py_roots.extend(join(at, "src"));
                        for r in get("roots")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                        {
                            if let Some(r) = r.as_str().and_then(|r| join(at, r)) {
                                d.py_roots.push(r);
                            }
                        }
                        if let Some(n) = get("name").and_then(|v| v.as_str()) {
                            d.py_dists.entry(py_name(n)).or_insert_with(|| path.clone());
                        }
                    }
                    "manifest-php/composer" => {
                        for (key, out) in [("psr-4", &mut d.psr4), ("psr-0", &mut d.psr0)] {
                            for (prefix, dirs) in
                                get(key).and_then(|v| v.as_object()).into_iter().flatten()
                            {
                                for dir in dirs
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|x| x.as_str())
                                {
                                    if let Some(f) = join(at, dir) {
                                        out.push((prefix.clone(), f));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Longest first, then by name: one answer whatever the manifests' order.
            d.go.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
            d.psr4
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
            d.psr0
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
            d.py_roots.sort();
            d.py_roots.dedup();
            self.derived = Some(d);
        }
        self.derived.as_ref().expect("just built")
    }

    /// Resolves one reference of a Go, Python or PHP file. Every path probed is logged on `st`.
    pub fn resolve(&mut self, kind: &str, st: &mut FsState, from: &str, spec: &str) -> Outcome {
        let d = self.derived();
        if kind.starts_with("go-imports") {
            go(d, st, from, spec)
        } else if kind.starts_with("py-imports") {
            py(d, st, from, spec)
        } else {
            php(d, st, from, spec)
        }
    }
}

fn file(st: &mut FsState, p: &str) -> bool {
    st.note(p);
    st.files.contains_key(p)
}

fn link(from: &str, to: String) -> Outcome {
    Outcome::Link((LinkKind::Import, from.to_owned(), to))
}

fn go(d: &Derived, st: &mut FsState, from: &str, spec: &str) -> Outcome {
    st.note("\0go");
    let hit = d.go.iter().find(|(m, _)| {
        spec == m
            || spec
                .strip_prefix(m.as_str())
                .is_some_and(|r| r.starts_with('/'))
    });
    match hit {
        Some((m, folder)) => {
            let Some(target) = join(folder, spec[m.len()..].trim_start_matches('/')) else {
                return Outcome::Unresolved;
            };
            st.note(&target);
            if st.is_dir(&target) {
                link(from, format!("{target}/"))
            } else {
                Outcome::Unresolved
            }
        }
        None if spec.split('/').next().is_some_and(|s| s.contains('.')) => Outcome::External,
        None => Outcome::Builtin,
    }
}

fn py(d: &Derived, st: &mut FsState, from: &str, spec: &str) -> Outcome {
    st.note("\0py");
    let (module, name) = match spec.split_once(':') {
        Some((m, n)) => (m, Some(n).filter(|n| *n != "*")),
        None => (spec, None),
    };
    let dots = module.chars().take_while(|c| *c == '.').count();
    let rest = module[dots..].replace('.', "/");
    let bases: Vec<String> = if dots > 0 {
        let mut b = dir(from).to_owned();
        for _ in 1..dots {
            match join(&b, "..") {
                Some(up) => b = up,
                None => return Outcome::Unresolved,
            }
        }
        vec![b]
    } else {
        // Roots holding the importer first (nearest first), then the rest, then its top folder and
        // the root.
        let mut near: Vec<&String> = d
            .py_roots
            .iter()
            .filter(|r| r.is_empty() || from.starts_with(&format!("{r}/")))
            .collect();
        near.sort_by_key(|r| std::cmp::Reverse(r.len()));
        let mut v: Vec<String> = near.into_iter().cloned().collect();
        v.extend(d.py_roots.iter().cloned());
        v.push(from.split_once('/').map_or("", |(t, _)| t).to_owned());
        v.push(String::new());
        let mut seen = std::collections::HashSet::new();
        v.retain(|x| seen.insert(x.clone()));
        v
    };
    for base in &bases {
        let Some(full) = join(base, &rest) else {
            continue;
        };
        if let Some(n) = name {
            let Some(sub) = join(&full, &n.replace('.', "/")) else {
                continue;
            };
            for p in [format!("{sub}.py"), format!("{sub}/__init__.py")] {
                if file(st, &p) {
                    return link(from, p);
                }
            }
        }
        if !rest.is_empty() || dots > 0 {
            let mut probes = vec![format!("{full}/__init__.py")];
            if !rest.is_empty() {
                probes.insert(0, format!("{full}.py"));
            }
            for p in probes {
                if file(st, p.trim_start_matches('/')) {
                    return link(from, p.trim_start_matches('/').to_owned());
                }
            }
        }
    }
    if dots > 0 {
        return Outcome::Unresolved;
    }
    let top = py_name(module.split('.').next().unwrap_or(module));
    match d.py_dists.get(&top) {
        Some(p) => Outcome::Link((LinkKind::Package, from.to_owned(), p.clone())),
        None => Outcome::External,
    }
}

fn php(d: &Derived, st: &mut FsState, from: &str, spec: &str) -> Outcome {
    st.note("\0php");
    if spec.ends_with(".php") || spec.contains('/') {
        // A literal include, relative to the file's folder; an absolute path is never followed.
        if spec.starts_with('/') {
            return Outcome::Unresolved;
        }
        return match join(dir(from), spec) {
            Some(p) if file(st, &p) => link(from, p),
            _ => Outcome::Unresolved,
        };
    }
    let class = spec.trim_start_matches('\\');
    let mut matched = false;
    for (prefix, folder) in &d.psr4 {
        if let Some(rest) = class.strip_prefix(prefix.as_str()) {
            matched = true;
            if let Some(p) = join(folder, &format!("{}.php", rest.replace('\\', "/")))
                && file(st, &p)
            {
                return link(from, p);
            }
        }
    }
    for (prefix, folder) in &d.psr0 {
        if class.starts_with(prefix.as_str()) {
            matched = true;
            // PSR-0: namespace separators are folders, and so are underscores in the class name.
            let (ns, name) = class.rsplit_once('\\').unwrap_or(("", class));
            let rel = if ns.is_empty() {
                name.replace('_', "/")
            } else {
                format!("{}/{}", ns.replace('\\', "/"), name.replace('_', "/"))
            };
            if let Some(p) = join(folder, &format!("{rel}.php"))
                && file(st, &p)
            {
                return link(from, p);
            }
        }
    }
    if matched {
        Outcome::Unresolved
    } else {
        Outcome::External
    }
}
