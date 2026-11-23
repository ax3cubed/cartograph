//! Which branch of a repo is indexed (C3; IDX-09). Never touches the network: no `git remote show`,
//! only refs and config already on disk. Resolved once at init and stored in the manifest.

use std::path::Path;

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::DiscoveryError;

/// Which rung of the C3 ladder gave the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `default_branch` in cartograph.yaml.
    Manifest,
    /// `refs/remotes/origin/HEAD`.
    OriginHead,
    /// `init.defaultBranch` from user or system config, and that branch exists.
    InitDefaultBranch,
    Main,
    Master,
    /// The checked-out branch. The caller warns: it may be a feature branch.
    CurrentBranch,
}

/// The default branch of the repo at `root`, with where it came from.
pub fn default_branch(
    root: &Path,
    manifest: Option<&str>,
) -> Result<(String, Source), DiscoveryError> {
    let repo = gix::open_opts(root, gix::open::Options::isolated()).map_err(|e| {
        DiscoveryError::Branch {
            repo: root.display().to_string(),
            why: e.to_string(),
        }
    })?;
    // User and system config only: the repo's own config can't pick the branch (IDX-09).
    let init_default = gix::config::File::from_globals()
        .ok()
        .and_then(|c| c.string("init.defaultBranch"))
        .and_then(|s| s.to_str().ok().map(str::to_owned));
    default_branch_with(&repo, manifest, init_default.as_deref())
}

/// A branch name git would accept, so a value from a config or a manifest can't name `../x`.
pub fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && gix::validate::reference::name_partial(name.as_bytes().as_bstr()).is_ok()
}

fn has_branch(repo: &gix::Repository, name: &str) -> bool {
    valid_branch(name)
        && repo
            .try_find_reference(format!("refs/heads/{name}").as_str())
            .is_ok_and(|r| r.is_some())
}

/// The branch a symbolic ref points at, with `prefix` removed.
fn symbolic(repo: &gix::Repository, reference: &str, prefix: &str) -> Option<String> {
    let r = repo.try_find_reference(reference).ok()??;
    match &r.inner.target {
        gix::refs::Target::Symbolic(name) => name
            .as_bstr()
            .to_str()
            .ok()?
            .strip_prefix(prefix)
            .map(str::to_owned),
        gix::refs::Target::Object(_) => None,
    }
}

pub(crate) fn default_branch_with(
    repo: &gix::Repository,
    manifest: Option<&str>,
    init_default: Option<&str>,
) -> Result<(String, Source), DiscoveryError> {
    let shown = || {
        repo.workdir()
            .unwrap_or(repo.git_dir())
            .display()
            .to_string()
    };
    if let Some(b) = manifest {
        if !valid_branch(b) {
            return Err(DiscoveryError::Branch {
                repo: shown(),
                why: format!("default_branch `{b}` in cartograph.yaml isn't a valid branch name"),
            });
        }
        return Ok((b.to_owned(), Source::Manifest));
    }
    // The indexer walks the local branch or else origin's remote-tracking one; a stale origin/HEAD
    // (its branch pruned, or renamed upstream) names neither, so the ladder goes on.
    if let Some(b) = symbolic(repo, "refs/remotes/origin/HEAD", "refs/remotes/origin/")
        .filter(|b| valid_branch(b))
        .filter(|b| {
            has_branch(repo, b)
                || repo
                    .try_find_reference(format!("refs/remotes/origin/{b}").as_str())
                    .is_ok_and(|r| r.is_some())
        })
    {
        return Ok((b, Source::OriginHead));
    }
    if let Some(b) = init_default.filter(|b| has_branch(repo, b)) {
        return Ok((b.to_owned(), Source::InitDefaultBranch));
    }
    for (b, s) in [("main", Source::Main), ("master", Source::Master)] {
        if has_branch(repo, b) {
            return Ok((b.to_owned(), s));
        }
    }
    if let Some(b) = symbolic(repo, "HEAD", "refs/heads/").filter(|b| valid_branch(b)) {
        return Ok((b, Source::CurrentBranch));
    }
    Err(DiscoveryError::Branch {
        repo: shown(),
        why: "HEAD is detached and there is no origin/HEAD, main or master".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{git, repo_with_commit};

    fn resolve(dir: &Path, manifest: Option<&str>, init: Option<&str>) -> (String, Source) {
        let repo = gix::open_opts(dir, gix::open::Options::isolated()).unwrap();
        default_branch_with(&repo, manifest, init).unwrap()
    }

    #[test]
    fn each_rung_of_the_ladder() {
        let t = tempfile::tempdir().unwrap();
        let d = &repo_with_commit(t.path(), "r", "work");
        // 5: only the current branch.
        assert_eq!(
            resolve(d, None, None),
            ("work".into(), Source::CurrentBranch)
        );
        // 4: master, then main above it.
        git(d, &["branch", "master"]);
        assert_eq!(resolve(d, None, None), ("master".into(), Source::Master));
        git(d, &["branch", "main"]);
        assert_eq!(resolve(d, None, None), ("main".into(), Source::Main));
        // 3: init.defaultBranch only when that branch exists.
        assert_eq!(resolve(d, None, Some("trunk")).1, Source::Main);
        git(d, &["branch", "trunk"]);
        assert_eq!(
            resolve(d, None, Some("trunk")),
            ("trunk".into(), Source::InitDefaultBranch)
        );
        // 2: origin/HEAD.
        git(
            d,
            &["update-ref", "refs/remotes/origin/release/2026", "HEAD"],
        );
        git(
            d,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/release/2026",
            ],
        );
        assert_eq!(
            resolve(d, None, Some("trunk")),
            ("release/2026".into(), Source::OriginHead)
        );
        // 1: the manifest.
        assert_eq!(
            resolve(d, Some("develop"), Some("trunk")),
            ("develop".into(), Source::Manifest)
        );
    }

    #[test]
    fn a_stale_origin_head_falls_through() {
        let t = tempfile::tempdir().unwrap();
        let d = &repo_with_commit(t.path(), "r", "main");
        // origin renamed master to main and `fetch --prune` removed origin/master, but origin/HEAD
        // still points at it: nothing to index there.
        git(
            d,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/master",
            ],
        );
        assert_eq!(resolve(d, None, None), ("main".into(), Source::Main));
        // Once origin/master exists again, origin/HEAD wins, local branch or not.
        git(d, &["update-ref", "refs/remotes/origin/master", "HEAD"]);
        assert_eq!(
            resolve(d, None, None),
            ("master".into(), Source::OriginHead)
        );
    }

    #[test]
    fn slashes_and_non_ascii_names() {
        let t = tempfile::tempdir().unwrap();
        let d = &repo_with_commit(t.path(), "r", "feature/équipe-ü");
        assert_eq!(
            resolve(d, None, None),
            ("feature/équipe-ü".into(), Source::CurrentBranch)
        );
    }

    #[test]
    fn a_detached_head_with_nothing_else_names_the_manifest_key() {
        let t = tempfile::tempdir().unwrap();
        let d = &repo_with_commit(t.path(), "r", "work");
        git(d, &["checkout", "-q", "--detach"]);
        git(d, &["branch", "-D", "work"]);
        let repo = gix::open_opts(d, gix::open::Options::isolated()).unwrap();
        let e = default_branch_with(&repo, None, None).unwrap_err();
        assert!(cartograph_core::Explain::action(&e).contains("default_branch"));
    }

    #[test]
    fn invalid_names_are_refused() {
        for b in ["", "-x", "a..b", "a b", "x.lock", "a~1", "/x"] {
            assert!(!valid_branch(b), "{b:?}");
        }
        let t = tempfile::tempdir().unwrap();
        let d = &repo_with_commit(t.path(), "r", "work");
        let repo = gix::open_opts(d, gix::open::Options::isolated()).unwrap();
        assert!(default_branch_with(&repo, Some("../../etc"), None).is_err());
    }
}
