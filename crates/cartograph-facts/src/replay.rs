//! The replay engine (IDX-06): the production form of bench.mjs `makeIndex()`.
//!
//! Steps are applied oldest first. Each holds the tracked paths a commit changed, with their new
//! facts. A reference (file, specifier) is resolved by oxc_resolver over [`ReplayFs`], which logs
//! every path the resolution asked about; those paths are the keys it waits on (the reverse index).
//! A step re-resolves only the references waiting on a key whose existence or served content
//! changed, plus the new references. Each link counts its supporting references, and an interval
//! `[from, to)` opens when the count leaves zero and closes when it returns there. A link that
//! closes and reopens within one step is left open.
//!
//! ponytail: paths and keys are strings, not interned integers (a key is one `Arc<str>` shared by
//! every reference waiting on it), and resolutions aren't memoised across references; remotion
//! replays in a few seconds without either.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use cartograph_parse::manifest::{is_package_json, resolution_json};
use cartograph_parse::{BlobFacts, Fact, extractor_for};
use oxc_resolver::{ResolveError, ResolveOptions, ResolverGeneric, TsconfigDiscovery};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::fs::{FsState, ReplayFs, to_abs, to_rel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LinkKind {
    /// File to file.
    Import,
    /// File to a workspace package's package.json, for a bare specifier the resolver can't follow
    /// into the package (its entry point is a build output, not in git).
    Package,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Package => "package",
        }
    }
}

/// (kind, from path, to path).
pub type Link = (LinkKind, String, String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval {
    pub link: Link,
    pub from: u64,
    pub to: u64,
}

/// What one reference resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Link(Link),
    /// A bare specifier with no workspace package of that name: an npm dependency.
    External,
    /// A Node built-in, or a path the browser field ignores.
    Builtin,
    Unresolved,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub steps: u64,
    pub resolutions: u64,
    /// The most references one step re-resolved.
    pub max_in_step: usize,
}

struct Ref {
    from: String,
    spec: String,
    outcome: Outcome,
    keys: Vec<Arc<str>>,
}

/// One changed path and its new facts (`None`: gone).
pub type Change = (String, Option<Arc<BlobFacts>>);

pub fn resolve_options() -> ResolveOptions {
    ResolveOptions {
        extensions: [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json"]
            .map(String::from)
            .to_vec(),
        // The TS ESM rule: "./x.js" names x.ts.
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

pub struct Replay {
    fs: Arc<Mutex<FsState>>,
    resolver: ResolverGeneric<ReplayFs>,
    tree: HashMap<String, Arc<BlobFacts>>,
    refs: Vec<Option<Ref>>,
    free: Vec<usize>,
    by_file: FxHashMap<String, Vec<usize>>,
    /// The reverse index: key -> references waiting on it.
    waiting: FxHashMap<Arc<str>, FxHashSet<usize>>,
    /// Package name -> the package.json paths declaring it.
    pkgs: FxHashMap<String, BTreeSet<String>>,
    counts: FxHashMap<Link, u32>,
    open_at: FxHashMap<Link, u64>,
    touched: HashMap<Link, bool>,
    /// Intervals closed so far; the writer drains them.
    pub closed: Vec<Interval>,
    pub stats: Stats,
    reverse: bool,
}

impl Default for Replay {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Replay {
    /// `reverse = false` is the research's ablation: every live reference is re-resolved on any
    /// step that changes a key.
    pub fn new(reverse: bool) -> Self {
        let fs = Arc::new(Mutex::new(FsState::default()));
        let resolver =
            ResolverGeneric::new_with_file_system(ReplayFs(Arc::clone(&fs)), resolve_options());
        Self {
            fs,
            resolver,
            tree: HashMap::new(),
            refs: Vec::new(),
            free: Vec::new(),
            by_file: FxHashMap::default(),
            waiting: FxHashMap::default(),
            pkgs: FxHashMap::default(),
            counts: FxHashMap::default(),
            open_at: FxHashMap::default(),
            touched: HashMap::new(),
            closed: Vec::new(),
            stats: Stats::default(),
            reverse,
        }
    }

    /// The links live after the last step.
    pub fn links(&self) -> BTreeSet<Link> {
        self.counts.keys().cloned().collect()
    }

    /// Open links and the step each opened at.
    pub fn open(&self) -> impl Iterator<Item = (&Link, u64)> {
        self.open_at.iter().map(|(l, s)| (l, *s))
    }

    pub fn tree(&self) -> &HashMap<String, Arc<BlobFacts>> {
        &self.tree
    }

    /// Live references by outcome: (links, external, builtin, unresolved).
    pub fn outcomes(&self) -> (usize, usize, usize, usize) {
        let mut n = (0, 0, 0, 0);
        for r in self.refs.iter().flatten() {
            match r.outcome {
                Outcome::Link(_) => n.0 += 1,
                Outcome::External => n.1 += 1,
                Outcome::Builtin => n.2 += 1,
                Outcome::Unresolved => n.3 += 1,
            }
        }
        n
    }

    pub fn apply(&mut self, changes: &[Change], seq: u64) {
        self.stats.steps += 1;
        let pending = self.stage(changes);
        if pending.len() > 100_000 {
            tracing::warn!(
                seq,
                refs = pending.len(),
                "one step re-resolves many references"
            );
        }
        self.stats.max_in_step = self.stats.max_in_step.max(pending.len());
        // One cold start per importer: its references share the tsconfig and package.json reads.
        let batches: Vec<Vec<usize>> = pending
            .chunk_by(|a, b| {
                self.refs[*a].as_ref().map(|r| &r.from) == self.refs[*b].as_ref().map(|r| &r.from)
            })
            .map(<[usize]>::to_vec)
            .collect();
        for batch in batches {
            self.resolve_batch(&batch);
        }
        self.settle_step(seq);
    }

    /// Takes `changes` into the tree, the file system and the references, and returns the live
    /// references to resolve again, sorted by importer then specifier. Resolves nothing.
    fn stage(&mut self, changes: &[Change]) -> Vec<usize> {
        let mut dirty: Vec<String> = Vec::new();
        let mut pending: Vec<usize> = Vec::new();
        for (path, new) in changes {
            let old = self.tree.get(path).cloned();
            if match (&old, new) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                (None, None) => true,
                _ => false,
            } {
                continue;
            }
            let Some((_, kind)) = extractor_for(path) else {
                continue;
            };
            match new {
                Some(f) => self.tree.insert(path.clone(), Arc::clone(f)),
                None => self.tree.remove(path),
            };
            let package = is_package_json(path);
            let manifest = kind.starts_with("manifest-js");
            {
                let mut st = self.fs.lock().expect("not poisoned");
                let served: Option<Arc<str>> = new
                    .as_ref()
                    .filter(|_| manifest)
                    .map(|f| Arc::from(resolution_json(f.facts(), package)));
                if old.is_some() != new.is_some() {
                    dirty.push(path.clone());
                    st.count_dirs(path, if new.is_some() { 1 } else { -1 }, &mut dirty);
                } else if manifest && st.files.get(path) != Some(&served) {
                    dirty.push(path.clone());
                }
                if new.is_some() {
                    st.files.insert(path.clone(), served);
                } else {
                    st.files.remove(path);
                }
            }
            let (before, after) = (
                old.as_deref().map_or(&[][..], BlobFacts::facts),
                new.as_deref().map_or(&[][..], BlobFacts::facts),
            );
            if package {
                let (a, b) = (package_name(before), package_name(after));
                if a != b {
                    if let Some(a) = a {
                        self.pkgs.entry(a.clone()).or_default().remove(path);
                        self.relink(&a, &mut dirty);
                    }
                    if let Some(b) = b {
                        self.pkgs.entry(b.clone()).or_default().insert(path.clone());
                        self.relink(&b, &mut dirty);
                    }
                }
            } else if !manifest {
                let (a, b) = (specs(before), specs(after));
                for s in a.difference(&b) {
                    self.drop_ref(path, s);
                }
                for s in b.difference(&a) {
                    pending.push(self.add_ref(path, s));
                }
            }
        }
        if self.reverse {
            for k in &dirty {
                if let Some(w) = self.waiting.get(k.as_str()) {
                    pending.extend(w);
                }
            }
        } else if !dirty.is_empty() {
            pending.extend((0..self.refs.len()).filter(|i| self.refs[*i].is_some()));
        }
        pending.sort_unstable();
        pending.dedup();
        pending.retain(|id| self.refs[*id].is_some());
        pending.sort_by(|a, b| {
            let (x, y) = (self.refs[*a].as_ref(), self.refs[*b].as_ref());
            x.map(|r| (&r.from, &r.spec))
                .cmp(&y.map(|r| (&r.from, &r.spec)))
        });
        pending
    }

    /// Opens and closes intervals for the net effect of the step only.
    fn settle_step(&mut self, seq: u64) {
        for (link, was) in std::mem::take(&mut self.touched) {
            let is = self.counts.contains_key(&link);
            if was && !is {
                let from = self
                    .open_at
                    .remove(&link)
                    .expect("an open link has a start");
                self.closed.push(Interval {
                    link,
                    from,
                    to: seq,
                });
            } else if !was && is {
                self.open_at.insert(link, seq);
            }
        }
    }

    /// Each live reference's outcome and the reverse index, encoded. With the tree, that is
    /// everything [`Replay::restore`] needs to continue without resolving the tree again. Only the
    /// reverse-index replay keeps `waiting`, so only it has a snapshot.
    pub fn snapshot(&self) -> Vec<u8> {
        assert!(
            self.reverse,
            "the ablation keeps no reverse index to snapshot"
        );
        let mut strings = Interner::default();
        let mut position = vec![u32::MAX; self.refs.len()];
        let mut refs = Vec::new();
        for (id, r) in self.refs.iter().enumerate() {
            let Some(r) = r else { continue };
            position[id] = u32::try_from(refs.len()).expect("under 4G references");
            refs.push(SnapRef {
                from: strings.id(&r.from),
                spec: strings.id(&r.spec),
                outcome: match &r.outcome {
                    Outcome::Link((LinkKind::Import, _, to)) => SnapOutcome::Import(strings.id(to)),
                    Outcome::Link((LinkKind::Package, _, to)) => {
                        SnapOutcome::Package(strings.id(to))
                    }
                    Outcome::External => SnapOutcome::External,
                    Outcome::Builtin => SnapOutcome::Builtin,
                    Outcome::Unresolved => SnapOutcome::Unresolved,
                },
            });
        }
        let waiting = self
            .waiting
            .iter()
            .map(|(k, ids)| (strings.id(k), ids.iter().map(|i| position[*i]).collect()))
            .collect();
        postcard::to_stdvec(&Snapshot {
            strings: strings.list,
            refs,
            waiting,
        })
        .expect("a snapshot always encodes")
    }

    /// The replay at step `at` over `tree` (every live path and its facts), with the outcomes and
    /// reverse index of a [`Replay::snapshot`] taken at that step instead of resolving every
    /// reference. Every link opens at `at`. `None` when the bytes don't decode or don't hold
    /// exactly the tree's references: the caller replays the tree instead.
    pub fn restore(tree: &[Change], at: u64, snapshot: &[u8]) -> Option<Self> {
        let snap: Snapshot = postcard::from_bytes(snapshot).ok()?;
        let s = |i: u32| snap.strings.get(i as usize).map(String::as_str);
        let mut saved: FxHashMap<(&str, &str), usize> =
            FxHashMap::with_capacity_and_hasher(snap.refs.len(), Default::default());
        for (p, r) in snap.refs.iter().enumerate() {
            saved.insert((s(r.from)?, s(r.spec)?), p);
        }
        let mut replay = Self::new(true);
        let pending = replay.stage(tree);
        if pending.len() != snap.refs.len() || saved.len() != snap.refs.len() {
            return None;
        }
        // Snapshot position -> reference id.
        let mut id_of = vec![0; snap.refs.len()];
        for id in pending {
            let r = replay.refs[id].as_ref()?;
            let p = *saved.get(&(r.from.as_str(), r.spec.as_str()))?;
            id_of[p] = id;
            let link =
                |kind, to: u32| Some(Outcome::Link((kind, r.from.clone(), s(to)?.to_owned())));
            let outcome = match snap.refs[p].outcome {
                SnapOutcome::Import(to) => link(LinkKind::Import, to)?,
                SnapOutcome::Package(to) => link(LinkKind::Package, to)?,
                SnapOutcome::External => Outcome::External,
                SnapOutcome::Builtin => Outcome::Builtin,
                SnapOutcome::Unresolved => Outcome::Unresolved,
            };
            if let Outcome::Link(l) = &outcome {
                replay.bump(l.clone(), 1);
            }
            replay.refs[id].as_mut()?.outcome = outcome;
        }
        let mut per_ref = vec![0usize; snap.refs.len()];
        for (_, ps) in &snap.waiting {
            for p in ps {
                *per_ref.get_mut(*p as usize)? += 1;
            }
        }
        for (p, n) in per_ref.into_iter().enumerate() {
            replay.refs[id_of[p]].as_mut()?.keys.reserve_exact(n);
        }
        replay.waiting.reserve(snap.waiting.len());
        for (k, ps) in &snap.waiting {
            let key: Arc<str> = Arc::from(s(*k)?);
            let mut ids = FxHashSet::with_capacity_and_hasher(ps.len(), Default::default());
            for p in ps {
                let id = *id_of.get(*p as usize)?;
                replay.refs[id].as_mut()?.keys.push(Arc::clone(&key));
                ids.insert(id);
            }
            replay.waiting.insert(key, ids);
        }
        replay.settle_step(at);
        Some(replay)
    }

    /// Points `node_modules/<name>` at the package with the smallest manifest path.
    fn relink(&mut self, name: &str, dirty: &mut Vec<String>) {
        let link = format!("node_modules/{name}");
        let target = self
            .pkgs
            .get(name)
            .and_then(|s| s.first())
            .map(|p| p.rsplit_once('/').map_or("", |(d, _)| d).to_owned());
        if target.is_none() {
            self.pkgs.remove(name);
        }
        let mut st = self.fs.lock().expect("not poisoned");
        if st.links.get(&link) == target.as_ref() {
            return;
        }
        dirty.push(link.clone());
        match target {
            Some(t) => {
                if st.links.insert(link.clone(), t).is_none() {
                    st.count_dirs(&link, 1, dirty);
                }
            }
            None => {
                st.links.remove(&link);
                st.count_dirs(&link, -1, dirty);
            }
        }
    }

    fn add_ref(&mut self, from: &str, spec: &str) -> usize {
        let r = Ref {
            from: from.to_owned(),
            spec: spec.to_owned(),
            outcome: Outcome::Unresolved,
            keys: Vec::new(),
        };
        let id = match self.free.pop() {
            Some(i) => {
                self.refs[i] = Some(r);
                i
            }
            None => {
                self.refs.push(Some(r));
                self.refs.len() - 1
            }
        };
        self.by_file.entry(from.to_owned()).or_default().push(id);
        id
    }

    fn drop_ref(&mut self, from: &str, spec: &str) {
        let ids = self.by_file.get_mut(from).expect("a file with refs");
        let at = ids
            .iter()
            .position(|i| self.refs[*i].as_ref().is_some_and(|r| r.spec == spec))
            .expect("the ref exists");
        let id = ids.swap_remove(at);
        if ids.is_empty() {
            self.by_file.remove(from);
        }
        let r = self.refs[id].take().expect("live");
        self.unwait(id, &r.keys);
        if let Outcome::Link(l) = r.outcome {
            self.bump(l, -1);
        }
        self.free.push(id);
    }

    fn unwait(&mut self, id: usize, keys: &[Arc<str>]) {
        for k in keys {
            if let Some(w) = self.waiting.get_mut(k) {
                w.remove(&id);
                if w.is_empty() {
                    self.waiting.remove(k);
                }
            }
        }
    }

    /// Resolves references of one importer. The resolver starts cold, so the file system log holds
    /// every path the batch read; a reference's keys are everything logged up to and including its
    /// own resolution, since it may have used what an earlier one in the batch loaded into the cache.
    /// That is a superset of what it depends on, which keeps the replay exact.
    fn resolve_batch(&mut self, ids: &[usize]) {
        self.resolver.clear_cache();
        self.fs.lock().expect("not poisoned").log = Some(Vec::new());
        let mut seen: Vec<String> = Vec::new();
        for &id in ids {
            let r = self.refs[id].as_ref().expect("live");
            let res = self.resolver.resolve_file(to_abs(&r.from), &r.spec);
            let mut st = self.fs.lock().expect("not poisoned");
            seen.append(st.log.as_mut().expect("recording"));
            drop(st);
            seen.sort_unstable();
            seen.dedup();
            let mut keys = seen.clone();
            let r = self.refs[id].as_ref().expect("live");
            let outcome = outcome(&r.from, &r.spec, res, &self.pkgs, &mut keys);
            keys.sort_unstable();
            keys.dedup();
            self.settle(id, outcome, keys);
        }
        self.fs.lock().expect("not poisoned").log = None;
    }

    fn settle(&mut self, id: usize, outcome: Outcome, keys: Vec<String>) {
        let mut r = self.refs[id].take().expect("live");
        self.stats.resolutions += 1;
        if self.reverse {
            self.unwait(id, &r.keys);
            r.keys = keys
                .into_iter()
                .map(|k| match self.waiting.get_key_value(k.as_str()) {
                    Some((shared, _)) => Arc::clone(shared),
                    None => Arc::from(k),
                })
                .collect();
            for k in &r.keys {
                self.waiting.entry(Arc::clone(k)).or_default().insert(id);
            }
        } else {
            r.keys = keys.into_iter().map(Arc::from).collect();
        }
        if outcome != r.outcome {
            if let Outcome::Link(l) = &r.outcome {
                self.bump(l.clone(), -1);
            }
            if let Outcome::Link(l) = &outcome {
                self.bump(l.clone(), 1);
            }
            r.outcome = outcome;
        }
        self.refs[id] = Some(r);
    }

    fn bump(&mut self, link: Link, d: i32) {
        let n = self.counts.get(&link).copied().unwrap_or(0);
        self.touched.entry(link.clone()).or_insert(n > 0);
        let m = n.checked_add_signed(d).expect("counts balance");
        if m == 0 {
            self.counts.remove(&link);
        } else {
            self.counts.insert(link, m);
        }
    }
}

/// Bump when what a snapshot means changes: the resolve options, how keys are recorded, or the
/// encoding. A snapshot of another version is not restored.
pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Snapshot {
    /// Every path, specifier and key once; the rest are indexes into it.
    strings: Vec<String>,
    refs: Vec<SnapRef>,
    /// The reverse index: each key and the positions in `refs` waiting on it.
    waiting: Vec<(u32, Vec<u32>)>,
}

#[derive(Serialize, Deserialize)]
struct SnapRef {
    from: u32,
    spec: u32,
    outcome: SnapOutcome,
}

/// An [`Outcome`] without its link's source, which is the reference's.
#[derive(Serialize, Deserialize)]
enum SnapOutcome {
    Import(u32),
    Package(u32),
    External,
    Builtin,
    Unresolved,
}

#[derive(Default)]
struct Interner<'a> {
    ids: HashMap<&'a str, u32>,
    list: Vec<String>,
}

impl<'a> Interner<'a> {
    fn id(&mut self, s: &'a str) -> u32 {
        *self.ids.entry(s).or_insert_with(|| {
            self.list.push(s.to_owned());
            u32::try_from(self.list.len() - 1).expect("under 4G strings")
        })
    }
}

fn specs(facts: &[Fact]) -> BTreeSet<&str> {
    facts
        .iter()
        .filter_map(|f| match f {
            Fact::Import { spec, .. } => Some(spec.as_str()),
            _ => None,
        })
        .collect()
}

/// The `name` of a package.json, if it is a usable package name.
fn package_name(facts: &[Fact]) -> Option<String> {
    let v = facts.iter().find_map(|f| match f {
        Fact::Manifest { field, value } if field == "name" => Some(value),
        _ => None,
    })?;
    let name: String = serde_json::from_str(v).ok()?;
    valid_package_name(&name).then_some(name)
}

/// Rejects names that would turn `node_modules/<name>` into a path elsewhere.
fn valid_package_name(n: &str) -> bool {
    let parts: Vec<&str> = n.split('/').collect();
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains('\\');
    match parts.as_slice() {
        [one] => ok(one) && !one.starts_with('@'),
        [scope, one] => scope.starts_with('@') && ok(&scope[1..]) && ok(one),
        _ => false,
    }
}

/// The package a bare specifier names: `@scope/pkg/sub` -> `@scope/pkg`.
fn bare_name(spec: &str) -> Option<&str> {
    if spec.starts_with(['.', '/', '#']) || spec.contains(':') {
        return None;
    }
    let mut it = spec.match_indices('/');
    let end = if spec.starts_with('@') {
        it.nth(1).map_or(spec.len(), |(i, _)| i)
    } else {
        it.next().map_or(spec.len(), |(i, _)| i)
    };
    Some(&spec[..end])
}

fn outcome(
    from: &str,
    spec: &str,
    res: Result<oxc_resolver::Resolution, ResolveError>,
    pkgs: &FxHashMap<String, BTreeSet<String>>,
    keys: &mut Vec<String>,
) -> Outcome {
    match res {
        Ok(r) => match to_rel(r.path()) {
            Some(t) => Outcome::Link((LinkKind::Import, from.to_owned(), t)),
            None => Outcome::Unresolved,
        },
        Err(ResolveError::Builtin { .. } | ResolveError::Ignored(_)) => Outcome::Builtin,
        Err(_) => match bare_name(spec) {
            Some(name) => {
                keys.push(format!("node_modules/{name}"));
                match pkgs.get(name).and_then(|s| s.first()) {
                    Some(p) => Outcome::Link((LinkKind::Package, from.to_owned(), p.clone())),
                    None => Outcome::External,
                }
            }
            None => Outcome::Unresolved,
        },
    }
}

/// The links of one tree computed from nothing: the oracle the replay is checked against. It shares
/// the file system and the resolution rule with the replay, and none of its incremental state.
pub fn from_scratch(tree: &HashMap<String, Arc<BlobFacts>>) -> BTreeSet<Link> {
    let mut st = FsState::default();
    let mut pkgs: FxHashMap<String, BTreeSet<String>> = FxHashMap::default();
    let mut scratch = Vec::new();
    for (path, f) in tree {
        let Some((_, kind)) = extractor_for(path) else {
            continue;
        };
        let package = is_package_json(path);
        let served = kind
            .starts_with("manifest-js")
            .then(|| Arc::from(resolution_json(f.facts(), package)));
        st.files.insert(path.clone(), served);
        st.count_dirs(path, 1, &mut scratch);
        if package && let Some(n) = package_name(f.facts()) {
            pkgs.entry(n).or_default().insert(path.clone());
        }
    }
    for (name, paths) in &pkgs {
        let first = paths.first().expect("non-empty");
        let link = format!("node_modules/{name}");
        st.links.insert(
            link.clone(),
            first.rsplit_once('/').map_or("", |(d, _)| d).to_owned(),
        );
        st.count_dirs(&link, 1, &mut scratch);
    }
    let resolver = ResolverGeneric::new_with_file_system(
        ReplayFs(Arc::new(Mutex::new(st))),
        resolve_options(),
    );
    let mut out = BTreeSet::new();
    for (path, f) in tree {
        if !extractor_for(path).is_some_and(|(_, k)| k.starts_with("ts-imports")) {
            continue;
        }
        for spec in specs(f.facts()) {
            let res = resolver.resolve_file(to_abs(path), spec);
            if let Outcome::Link(l) = outcome(path, spec, res, &pkgs, &mut scratch) {
                out.insert(l);
            }
        }
    }
    out
}
