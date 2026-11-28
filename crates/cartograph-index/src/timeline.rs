//! The merged multi-repo timeline (IDX-12): every repository's first-parent steps on one dense
//! sequence, ordered by corrected date, then repository name, then position in the repository.
//!
//! Paths are namespaced by repository (`<name>/<path>`) once a workspace has more than one, as
//! bench.mjs does, so the replay sees one tree. Package keys are workspace-wide: the replay links
//! `node_modules/<name>` at the root of that tree to the package.json with the smallest path, so a
//! bare import in one repository opens a link to another repository's package when it appears and
//! closes it when it goes. Two repositories declaring one name resolve to the first by
//! (repository name, path), deterministically, and [`duplicate_packages`] reports the name.
//!
//! The tie-break is the repository *name*, not its row id: names are what the workspace file
//! gives, so a repository added later sorts exactly where a full build would put it, and the
//! "first by (repo, path)" package rule above uses the same order.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use cartograph_git::CommitStep;
use cartograph_parse::manifest::is_package_json;
use cartograph_parse::{BlobFacts, Fact};

/// One repository of a workspace. `name` becomes its path prefix, so it is checked by
/// [`valid_repo_name`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSpec {
    pub name: String,
    pub path: PathBuf,
    /// The branch indexed (the manifest's `default_branch`, IDX-09); `None` follows HEAD.
    pub branch: Option<String>,
    /// Folder prefixes, relative to the repository root, that produce nodes and links; empty for
    /// the whole repository. package.json files outside them are still read, so package keys stay
    /// visible workspace-wide.
    pub scopes: Vec<String>,
}

impl RepoSpec {
    /// Whether the walk keeps `path` (repository-relative): tracked by an extractor, not excluded,
    /// and inside a scope or a package.json.
    pub fn keeps(&self, exclude: &[String], path: &str) -> bool {
        self.keeps_with(exclude, &[], path)
    }

    /// [`RepoSpec::keeps`], also keeping the YAML and JSON files the manifest's `contracts:` globs
    /// name (XS-08).
    pub fn keeps_with(&self, exclude: &[String], contracts: &[String], path: &str) -> bool {
        (cartograph_parse::tracked(path)
            || (cartograph_parse::openapi::is_data(path)
                && contracts.iter().any(|g| glob_matches(g, path))))
            && !excluded(exclude, path)
            && (self.scopes.is_empty()
                || is_package_json(path)
                || self.scopes.iter().any(|s| {
                    let s = s.trim_end_matches('/');
                    path == s || path.strip_prefix(s).is_some_and(|r| r.starts_with('/'))
                }))
    }
}

/// Whether a `files.exclude` glob matches `path` or one of its folders. `*` and `?` stay within a
/// segment, `**` spans any number of segments; patterns are relative to the repository root.
///
/// ponytail: no `{a,b}` alternatives or `[...]` classes, which VS Code also accepts; such a
/// pattern matches literally. A glob crate replaces this if workspaces use them.
pub fn excluded(patterns: &[String], path: &str) -> bool {
    let segs: Vec<&str> = path.split('/').collect();
    patterns.iter().any(|p| {
        let p = p.trim_start_matches("./").trim_end_matches('/');
        let pat: Vec<&str> = p.split('/').collect();
        (1..=segs.len()).any(|k| glob(&pat, &segs[..k]))
    })
}

/// Whether a workspace glob (`packages/*`, `apps/**`) matches the folder `path` exactly. Patterns
/// come from manifests in the repository, so they are capped: 256 bytes and 32 segments (IDX-20
/// security); a longer one matches nothing.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let p = pattern.trim_start_matches("./").trim_end_matches('/');
    if p.len() > 256 || p.split('/').count() > 32 {
        return false;
    }
    let pat: Vec<&str> = p.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    glob(&pat, &segs)
}

// Both matchers are greedy with one backtrack point, the last star: a later star can absorb
// anything an earlier one could, so nothing else needs retrying. The patterns come from a committed
// workspace file and run on every changed path under the writer lock, so the cost must stay
// polynomial whatever they hold (a recursive matcher is exponential in the number of stars).

/// Path segments against pattern segments: `**` spans any number of segments, any other pattern
/// segment matches exactly one.
fn glob(p: &[&str], s: &[&str]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match p.get(pi) {
            Some(&"**") => {
                star = Some((pi + 1, si));
                pi += 1;
            }
            Some(x) if segment(x.as_bytes(), s[si].as_bytes()) => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    star = Some((sp, ss + 1));
                    pi = sp;
                    si = ss + 1;
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|x| *x == "**")
}

/// One segment: `*` is any run of bytes, `?` any one byte, anything else itself.
fn segment(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match p.get(pi) {
            Some(b'*') => {
                star = Some((pi + 1, si));
                pi += 1;
            }
            Some(&c) if c == b'?' || c == s[si] => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    star = Some((sp, ss + 1));
                    pi = sp;
                    si = ss + 1;
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// A name usable as a path prefix: ASCII letters, digits, `.`, `_` and `-`, at most 64 characters,
/// not hidden, and not a folder name the extractors skip (a repo named `dist` would hide its files).
pub fn valid_repo_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('.')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !matches!(n, "node_modules" | "dist" | "build" | "vendor")
}

/// `path` in the merged tree.
pub fn namespaced(prefix: Option<&str>, path: &str) -> String {
    match prefix {
        Some(p) => format!("{p}/{path}"),
        None => path.to_owned(),
    }
}

/// Merges per-repository step lists, each oldest first, into one timeline of
/// `(repository index, step)`. `per_repo` must be in name order: the index is the tie-break.
///
/// Each list's corrected times strictly increase, so `(corrected time, index)` is unique per step
/// and the order is total and keeps each repository's own order.
///
/// ponytail: a sort, not a k-way merge over streams, as in bench.mjs; 35,000 steps sort in a few
/// milliseconds. A streaming merge is needed only once walks are pipelined into the replay.
pub fn merge(per_repo: Vec<Vec<CommitStep>>) -> Vec<(usize, CommitStep)> {
    let mut all: Vec<(usize, CommitStep)> = per_repo
        .into_iter()
        .enumerate()
        .flat_map(|(r, steps)| steps.into_iter().map(move |s| (r, s)))
        .collect();
    // Stable, so equal keys (impossible within one repository) would keep their position.
    all.sort_by_key(|(r, s)| (s.corrected_time, *r));
    all
}

/// Package names declared by package.json files in more than one repository of `tree`, a merged
/// tree with namespaced paths. The first by (repository, path) wins the name; these are the losers'
/// names, recorded so a hostile repository can't silently capture another's links.
pub fn duplicate_packages(tree: &HashMap<String, Arc<BlobFacts>>) -> Vec<String> {
    let mut by_name: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (path, f) in tree {
        if !is_package_json(path) {
            continue;
        }
        let repo = path.split_once('/').map_or("", |(r, _)| r);
        for fact in f.facts() {
            if let Fact::Manifest { field, value } = fact
                && field == "name"
            {
                by_name.entry(value.as_str()).or_default().insert(repo);
            }
        }
    }
    by_name
        .into_iter()
        .filter(|(_, repos)| repos.len() > 1)
        .map(|(n, _)| n.trim_matches('"').to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{RepoSpec, excluded};

    #[test]
    fn exclude_globs_match_a_path_or_its_folders() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(excluded(&p(&["**/dist"]), "a/dist/x.ts"));
        assert!(excluded(&p(&["**/dist"]), "dist/x.ts"));
        assert!(excluded(&p(&["legacy"]), "legacy/a.ts"));
        assert!(!excluded(&p(&["legacy"]), "src/legacy/a.ts"));
        assert!(excluded(&p(&["**/*.gen.ts"]), "src/a.gen.ts"));
        assert!(excluded(&p(&["src/?.ts"]), "src/a.ts"));
        assert!(!excluded(&p(&["src/?.ts"]), "src/ab.ts"));
        assert!(!excluded(&p(&["**/dist"]), "src/distant.ts"));
    }

    #[test]
    fn scopes_keep_their_folders_and_every_package_json() {
        let r = RepoSpec {
            scopes: vec!["packages/auth".into()],
            ..Default::default()
        };
        assert!(r.keeps(&[], "packages/auth/src/a.ts"));
        assert!(!r.keeps(&[], "packages/authz/a.ts"));
        assert!(!r.keeps(&[], "apps/web/a.ts"));
        assert!(r.keeps(&[], "apps/web/package.json"));
        assert!(!r.keeps(&["**/src".into()], "packages/auth/src/a.ts"));
    }
}
