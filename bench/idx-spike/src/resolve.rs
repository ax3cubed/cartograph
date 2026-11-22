//! H1: oxc_resolver driven over an in-memory file system built from a git tree (no checkout).
//!
//! Questions: can a custom FileSystem be supplied; does resolve_with_context report
//! file_dependencies and missing_dependencies; and are those dependencies enough to drive replay,
//! i.e. if none of a resolution's dependencies changed between two commits, is its result the same?

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use gix::bstr::ByteSlice;
use oxc_resolver::{
    FileMetadata, FileSystem, ResolveContext, ResolveError, ResolveOptions, ResolverGeneric,
    TsconfigDiscovery,
};
use regex::Regex;

use crate::R;

const ROOT: &str = "/repo";
const SRC_EXT: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// A git tree as a read-only file system. Blob contents are read from the object database on demand.
pub struct TreeFs {
    files: HashMap<PathBuf, gix::ObjectId>,
    dirs: HashSet<PathBuf>,
    /// pnpm-style links: /repo/node_modules/<package name> -> the package's folder.
    links: HashMap<PathBuf, PathBuf>,
    repo: Mutex<gix::Repository>,
}

impl TreeFs {
    pub fn at(repo: &gix::Repository, tree: gix::ObjectId) -> R<Self> {
        let mut rec = gix::traverse::tree::Recorder::default();
        repo.find_tree(tree)?.traverse().breadthfirst(&mut rec)?;
        let mut files = HashMap::new();
        let mut dirs = HashSet::from([PathBuf::from(ROOT)]);
        for e in rec.records {
            let p = Path::new(ROOT).join(e.filepath.to_str_lossy().as_ref());
            if e.mode.is_tree() {
                dirs.insert(p);
            } else if e.mode.is_blob() || e.mode.is_link() {
                files.insert(p, e.oid);
            }
        }
        let mut fs = Self {
            files,
            dirs,
            links: HashMap::new(),
            repo: Mutex::new(repo.clone()),
        };
        // Workspace packages become node_modules links, as `pnpm install` would make them.
        let manifests: Vec<PathBuf> = fs
            .files
            .keys()
            .filter(|p| {
                p.ends_with("package.json")
                    && !p.components().any(|c| c.as_os_str() == "node_modules")
            })
            .cloned()
            .collect();
        let nm = Path::new(ROOT).join("node_modules");
        for m in manifests {
            let Ok(text) = fs.read_to_string(&m) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                let link = nm.join(name);
                if let Some(scope) = link.parent() {
                    fs.dirs.insert(scope.to_path_buf());
                }
                fs.links
                    .insert(link, m.parent().expect("a file has a parent").to_path_buf());
            }
        }
        if !fs.links.is_empty() {
            fs.dirs.insert(nm);
        }
        Ok(fs)
    }

    /// Follows links in any prefix of the path.
    fn real(&self, p: &Path) -> PathBuf {
        real_path(&self.links, p)
    }

    pub fn source_files(&self) -> Vec<(PathBuf, gix::ObjectId)> {
        let skip = ["node_modules", "dist", "build", "vendor", ".next"];
        let mut v: Vec<_> = self
            .files
            .iter()
            .filter(|(p, _)| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| SRC_EXT.contains(&e))
            })
            .filter(|(p, _)| {
                !p.components()
                    .any(|c| skip.iter().any(|s| c.as_os_str() == *s))
            })
            .map(|(p, id)| (p.clone(), *id))
            .collect();
        v.sort();
        v
    }
}

/// Follows links in any prefix of the path.
fn real_path(links: &HashMap<PathBuf, PathBuf>, p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
        for _ in 0..8 {
            match links.get(&out) {
                Some(target) => out = target.clone(),
                None => break,
            }
        }
    }
    out
}

impl FileSystem for TreeFs {
    fn new() -> Self {
        unreachable!("TreeFs is built with TreeFs::at and passed to new_with_file_system")
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let id = self
            .files
            .get(&self.real(path))
            .ok_or(io::ErrorKind::NotFound)?;
        let repo = self.repo.lock().expect("not poisoned");
        let blob = repo.find_blob(*id).map_err(io::Error::other)?;
        Ok(blob.data.clone())
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let p = self.real(path);
        if self.files.contains_key(&p) {
            Ok(FileMetadata::new(true, false, false))
        } else if self.dirs.contains(&p) {
            Ok(FileMetadata::new(false, true, false))
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if self.links.contains_key(path) {
            return Ok(FileMetadata::new(false, false, true));
        }
        self.metadata(path)
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        self.links
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput).into())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let p = self.real(path);
        if self.files.contains_key(&p) || self.dirs.contains(&p) {
            Ok(p)
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }
}

pub fn options() -> ResolveOptions {
    ResolveOptions {
        extensions: [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json"]
            .map(String::from)
            .to_vec(),
        extension_alias: vec![(
            ".js".into(),
            vec![".ts".into(), ".tsx".into(), ".js".into()],
        )],
        condition_names: ["types", "import", "require", "node", "default"]
            .map(String::from)
            .to_vec(),
        main_fields: vec!["types".into(), "module".into(), "main".into()],
        tsconfig: Some(TsconfigDiscovery::Auto),
        builtin_modules: true,
        ..ResolveOptions::default()
    }
}

/// Import specifiers, with the pattern bench.mjs uses, so the counts compare.
pub fn specifiers(re: &Regex, src: &str) -> Vec<String> {
    let mut seen: Vec<String> = re.captures_iter(src).map(|c| c[1].to_owned()).collect();
    seen.sort();
    seen.dedup();
    seen
}

pub fn import_regex() -> Regex {
    Regex::new(r#"(?:\bimport\s+(?:[^'"()]*?\sfrom\s+)?|\bexport\s+[^'"()]*?\sfrom\s+|\brequire\s*\(\s*|\bimport\s*\(\s*)['"]([^'"\n]+)['"]"#).expect("valid")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Internal(PathBuf),
    Builtin,
    NotFound,
    Error(String),
}

pub struct Resolved {
    pub outcome: Outcome,
    pub deps: Vec<PathBuf>,
    pub file_deps: usize,
    pub missing_deps: usize,
}

pub fn resolve_one(r: &ResolverGeneric<TreeFs>, importer: &Path, spec: &str) -> Resolved {
    let mut ctx = ResolveContext::default();
    let dir = importer.parent().expect("a file has a parent");
    let outcome = match r.resolve_with_context(dir, spec, None, &mut ctx) {
        Ok(res) => Outcome::Internal(res.path().to_path_buf()),
        Err(ResolveError::Builtin { .. }) => Outcome::Builtin,
        Err(ResolveError::NotFound(_)) => Outcome::NotFound,
        Err(e) => Outcome::Error(format!("{e:?}").chars().take(80).collect()),
    };
    let (file_deps, missing_deps) = (ctx.file_dependencies.len(), ctx.missing_dependencies.len());
    let mut deps: Vec<PathBuf> = ctx
        .file_dependencies
        .into_iter()
        .chain(ctx.missing_dependencies)
        .collect();
    deps.sort();
    Resolved {
        outcome,
        deps,
        file_deps,
        missing_deps,
    }
}

fn extract(repo: &gix::Repository, fs: &TreeFs, re: &Regex) -> R<Vec<(PathBuf, String)>> {
    let mut refs = Vec::new();
    for (path, id) in fs.source_files() {
        let blob = repo.find_blob(id)?;
        let Ok(src) = std::str::from_utf8(&blob.data) else {
            continue;
        };
        for s in specifiers(re, src) {
            refs.push((path.clone(), s));
        }
    }
    Ok(refs)
}

/// Resolves every import at one commit and reports what oxc_resolver says about dependencies.
pub fn at_commit(repo: &gix::Repository, rev: &str, runs: usize) -> R<serde_json::Value> {
    let commit = repo.rev_parse_single(rev)?.object()?.into_commit();
    let tree = commit.tree_id()?.detach();
    let t = Instant::now();
    let fs = TreeFs::at(repo, tree)?;
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let files = fs.source_files().len();
    let links = fs.links.len();
    let re = import_regex();
    let refs = extract(repo, &fs, &re)?;
    let resolver = ResolverGeneric::new_with_file_system(fs, options());

    let mut times = Vec::new();
    let mut summary = serde_json::Value::Null;
    for run in 0..runs {
        resolver.clear_cache();
        let t = Instant::now();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let (mut fd, mut md, mut with_missing, mut no_deps) = (0usize, 0usize, 0usize, 0usize);
        let mut errors: BTreeMap<String, usize> = BTreeMap::new();
        for (importer, spec) in &refs {
            let r = resolve_one(&resolver, importer, spec);
            fd += r.file_deps;
            md += r.missing_deps;
            with_missing += usize::from(r.missing_deps > 0);
            no_deps += usize::from(r.deps.is_empty());
            let k = match &r.outcome {
                Outcome::Internal(p) if p.starts_with(ROOT) => "resolved",
                Outcome::Internal(_) => "resolved_outside",
                Outcome::Builtin => "builtin",
                Outcome::NotFound => "not_found",
                Outcome::Error(e) => {
                    *errors
                        .entry(e.split('(').next().unwrap_or("").to_owned())
                        .or_default() += 1;
                    "error"
                }
            };
            *counts.entry(k).or_default() += 1;
        }
        times.push(t.elapsed().as_secs_f64() * 1e3);
        if run == 0 {
            let n = refs.len().max(1) as f64;
            summary = serde_json::json!({
                "outcomes": counts,
                "errors": errors,
                "mean_file_dependencies": fd as f64 / n,
                "mean_missing_dependencies": md as f64 / n,
                "with_missing_dependencies": with_missing,
                "with_no_dependencies": no_deps,
            });
        }
    }
    Ok(serde_json::json!({
        "commit": commit.id.to_string(),
        "source_files": files,
        "workspace_package_links": links,
        "resolutions": refs.len(),
        "tree_fs_build_ms": build_ms,
        "resolve_ms_cold_cache": times,
        "per_resolution_us_median": crate::median(&times) * 1e3 / refs.len().max(1) as f64,
        "summary": summary,
    }))
}

/// Replay soundness: for pairs of commits, a resolution whose dependencies saw no change must give
/// the same answer at both commits. Counts violations (which would break IDX-06's reverse index).
pub fn replay_check(
    repo: &gix::Repository,
    tip: &str,
    pairs: usize,
    gap: usize,
) -> R<serde_json::Value> {
    let chain = crate::walk::first_parent_chain(repo, tip)?;
    let re = import_regex();
    let step = (chain.len().saturating_sub(gap)) / pairs.max(1);
    let (mut checked, mut unaffected, mut affected, mut changed_answers, mut violations) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut unaffected_aug, mut affected_aug, mut violations_aug) = (0usize, 0usize, 0usize);
    let mut examples = Vec::new();
    let mut ms = Vec::new();
    for i in 0..pairs {
        let a = step * i + step / 2;
        let b = (a + gap).min(chain.len() - 1);
        let (fa, fb) = (TreeFs::at(repo, chain[a].1)?, TreeFs::at(repo, chain[b].1)?);
        // Every path whose presence or content differs, plus each folder that appeared or vanished.
        let mut changed: HashSet<PathBuf> = HashSet::new();
        for (p, id) in &fa.files {
            if fb.files.get(p) != Some(id) {
                changed.insert(p.clone());
            }
        }
        for p in fb.files.keys() {
            if !fa.files.contains_key(p) {
                changed.insert(p.clone());
            }
        }
        changed.extend(fa.dirs.symmetric_difference(&fb.dirs).cloned());
        for (l, t) in fa.links.iter().chain(fb.links.iter()) {
            if fa.links.get(l) != fb.links.get(l) {
                changed.insert(l.clone());
                changed.insert(t.clone());
            }
        }
        // Importers present with the same blob at both commits.
        let same: Vec<PathBuf> = fa
            .source_files()
            .into_iter()
            .filter(|(p, id)| fb.files.get(p) == Some(id))
            .map(|(p, _)| p)
            .collect();
        let mut refs = Vec::new();
        for p in same {
            let blob = repo.find_blob(fa.files[&p])?;
            if let Ok(src) = std::str::from_utf8(&blob.data) {
                refs.extend(specifiers(&re, src).into_iter().map(|s| (p.clone(), s)));
            }
        }
        let links_a = fa.links.clone();
        let manifest_files: HashSet<PathBuf> = fa
            .files
            .keys()
            .filter(|p| p.ends_with("package.json") || p.ends_with("tsconfig.json"))
            .cloned()
            .collect();
        // A manifest counts as changed only when a field resolution reads changed (not a version bump).
        let relevant_changed: HashSet<PathBuf> = manifest_files
            .iter()
            .chain(
                fb.files
                    .keys()
                    .filter(|p| p.ends_with("package.json") || p.ends_with("tsconfig.json")),
            )
            .filter(|m| changed.contains(*m))
            .filter(|m| {
                resolution_fields(repo, fa.files.get(*m))
                    != resolution_fields(repo, fb.files.get(*m))
            })
            .cloned()
            .collect();
        let (ra, rb) = (
            ResolverGeneric::new_with_file_system(fa, options()),
            ResolverGeneric::new_with_file_system(fb, options()),
        );
        let t = Instant::now();
        for (imp, spec) in &refs {
            let x = resolve_one(&ra, imp, spec);
            let y = resolve_one(&rb, imp, spec);
            checked += 1;
            // A dependency reported through a node_modules link counts at its real path too, and
            // a dependency below a link or folder that appeared or vanished counts as touched.
            let raw = x.deps.iter().any(|d| {
                d.ancestors().any(|a| changed.contains(a))
                    || changed.contains(&real_path(&links_a, d))
            });
            // The manifests oxc reads but does not report: the nearest package.json above each
            // dependency (its package's exports and main) and the nearest package.json and
            // tsconfig.json above the importer.
            let mut manifests: HashSet<&PathBuf> = HashSet::new();
            let nearest = |start: &Path, name: &str| {
                start
                    .ancestors()
                    .skip(1)
                    .map(|a| a.join(name))
                    .find(|m| manifest_files.contains(m))
            };
            for d in &x.deps {
                manifests.extend(
                    nearest(&real_path(&links_a, d), "package.json")
                        .and_then(|m| manifest_files.get(&m)),
                );
            }
            for name in ["package.json", "tsconfig.json"] {
                manifests.extend(nearest(imp, name).and_then(|m| manifest_files.get(&m)));
            }
            let augmented = raw || manifests.iter().any(|m| relevant_changed.contains(*m));
            if x.outcome != y.outcome {
                changed_answers += 1;
            }
            for (touched, aff, unaff, viol) in [
                (raw, &mut affected, &mut unaffected, &mut violations),
                (
                    augmented,
                    &mut affected_aug,
                    &mut unaffected_aug,
                    &mut violations_aug,
                ),
            ] {
                if touched {
                    *aff += 1;
                } else {
                    *unaff += 1;
                    if x.outcome != y.outcome {
                        *viol += 1;
                    }
                }
            }
            if !raw && x.outcome != y.outcome && examples.len() < 5 {
                examples.push(serde_json::json!({"importer": imp, "spec": spec, "a": format!("{:?}", x.outcome), "b": format!("{:?}", y.outcome), "deps_a": x.deps.iter().take(12).collect::<Vec<_>>(), "caught_by_manifests": augmented}));
            }
        }
        ms.push(t.elapsed().as_secs_f64() * 1e3);
    }
    Ok(serde_json::json!({
        "pairs": pairs,
        "gap_commits": gap,
        "resolutions_checked": checked,
        "deps_untouched": unaffected,
        "deps_touched_so_re_resolved": affected,
        "answers_that_changed": changed_answers,
        "violations_untouched_but_changed": violations,
        "with_manifests": {"deps_untouched": unaffected_aug, "deps_touched_so_re_resolved": affected_aug, "violations_untouched_but_changed": violations_aug},
        "examples": examples,
        "ms_per_pair_both_sides": ms,
    }))
}

/// The fields of a package.json or tsconfig.json that can change a resolution.
fn resolution_fields(repo: &gix::Repository, id: Option<&gix::ObjectId>) -> Option<String> {
    let blob = repo.find_blob(*id?).ok()?;
    let v: serde_json::Value =
        serde_json::from_slice(&blob.data).unwrap_or(serde_json::Value::Null);
    let keys = [
        "name",
        "main",
        "module",
        "types",
        "typings",
        "exports",
        "imports",
        "browser",
        "type",
        "extends",
        "compilerOptions",
        "references",
    ];
    let picked: serde_json::Map<String, serde_json::Value> = keys
        .iter()
        .filter_map(|k| Some(((*k).to_owned(), v.get(*k)?.clone())))
        .collect();
    Some(serde_json::Value::Object(picked).to_string())
}
