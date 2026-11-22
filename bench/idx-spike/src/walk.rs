//! B7: the first-parent walk with a tree diff per commit, in three implementations, and rename
//! detection (exact, then 50% similarity with a 1,000-candidate limit).

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use gix::bstr::ByteSlice;
use gix::diff::tree::recorder::Location;
use gix::diff::tree_with_rewrites::{ChangeRef, Options};
use gix::objs::TreeRefIter;

use crate::R;

/// What one commit changed, relative to its first parent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    pub commit: String,
    /// Every changed path (blobs, symlinks and submodules; no trees). A rename lists both ends.
    pub paths: BTreeSet<String>,
    /// Detected renames, `(from, to)`.
    pub renames: BTreeSet<(String, String)>,
}

/// Oldest first: (commit, its tree, first parent's tree).
pub fn first_parent_chain(
    repo: &gix::Repository,
    tip: &str,
) -> R<Vec<(gix::ObjectId, gix::ObjectId, Option<gix::ObjectId>)>> {
    let tip = repo.rev_parse_single(tip)?.detach();
    let mut out = Vec::new();
    for info in repo.rev_walk([tip]).first_parent_only().all()? {
        let info = info?;
        let commit = repo.find_commit(info.id)?;
        out.push((info.id, commit.tree_id()?.detach()));
    }
    out.reverse();
    let mut chain = Vec::with_capacity(out.len());
    let mut prev: Option<gix::ObjectId> = None;
    for (id, tree) in out {
        chain.push((id, tree, prev));
        prev = Some(tree);
    }
    Ok(chain)
}

pub fn rewrites() -> gix::diff::Rewrites {
    gix::diff::Rewrites {
        copies: None,
        percentage: Some(0.5),
        limit: 1000,
        track_empty: false,
    }
}

/// gix: one tree diff per commit, rewrites optional. Reuses one resource cache and diff state.
pub fn gix_changes(repo: &gix::Repository, tip: &str, renames: bool) -> R<Vec<Changes>> {
    let chain = first_parent_chain(repo, tip)?;
    let mut cache = repo.diff_resource_cache_for_tree_diff()?;
    let mut state = gix::diff::tree::State::default();
    let empty = repo.empty_tree();
    let mut all = Vec::with_capacity(chain.len());
    for (commit, tree, parent) in chain {
        let new = repo.find_tree(tree)?;
        let old = match parent {
            Some(p) => repo.find_tree(p)?,
            None => empty.clone(),
        };
        let mut c = Changes {
            commit: commit.to_string(),
            ..Default::default()
        };
        let opts = Options {
            location: Some(Location::Path),
            rewrites: renames.then(rewrites),
        };
        cache.clear_resource_cache_keep_allocation();
        gix::diff::tree_with_rewrites(
            TreeRefIter::from_bytes(&old.data, old.id.kind()),
            TreeRefIter::from_bytes(&new.data, new.id.kind()),
            &mut cache,
            &mut state,
            &repo.objects,
            |change| -> Result<_, gix::Error> {
                match change {
                    ChangeRef::Addition {
                        location,
                        entry_mode,
                        ..
                    }
                    | ChangeRef::Deletion {
                        location,
                        entry_mode,
                        ..
                    }
                    | ChangeRef::Modification {
                        location,
                        entry_mode,
                        ..
                    } => {
                        if !entry_mode.is_tree() {
                            c.paths.insert(location.to_str_lossy().into_owned());
                        }
                    }
                    ChangeRef::Rewrite {
                        source_location,
                        location,
                        entry_mode,
                        ..
                    } => {
                        if !entry_mode.is_tree() {
                            let (from, to) = (
                                source_location.to_str_lossy().into_owned(),
                                location.to_str_lossy().into_owned(),
                            );
                            c.paths.insert(from.clone());
                            c.paths.insert(to.clone());
                            c.renames.insert((from, to));
                        }
                    }
                }
                Ok(std::ops::ControlFlow::Continue(()))
            },
            opts,
        )?;
        all.push(c);
    }
    Ok(all)
}

/// libgit2: the same walk and diffs, rename detection through find_similar.
pub fn git2_changes(path: &str, tip: &str, renames: bool) -> R<Vec<Changes>> {
    let repo = git2::Repository::open(path)?;
    let mut walk = repo.revwalk()?;
    walk.push(repo.revparse_single(tip)?.id())?;
    walk.simplify_first_parent()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)?;
    let mut all = Vec::new();
    for id in walk {
        let commit = repo.find_commit(id?)?;
        let new = commit.tree()?;
        let old = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let mut diff = repo.diff_tree_to_tree(old.as_ref(), Some(&new), None)?;
        if renames {
            let mut f = git2::DiffFindOptions::new();
            f.renames(true).rename_threshold(50).rename_limit(1000);
            diff.find_similar(Some(&mut f))?;
        }
        let mut c = Changes {
            commit: commit.id().to_string(),
            ..Default::default()
        };
        for d in diff.deltas() {
            let old = d
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            let new = d
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            if d.status() == git2::Delta::Renamed {
                let (o, n) = (old.unwrap_or_default(), new.unwrap_or_default());
                c.paths.insert(o.clone());
                c.paths.insert(n.clone());
                c.renames.insert((o, n));
            } else {
                c.paths.extend(new.or(old));
            }
        }
        all.push(c);
    }
    Ok(all)
}

/// The research baseline: one streaming `git log --raw` (bench.mjs's walk), parsed.
pub fn git_log_changes(path: &str, tip: &str, renames: bool) -> R<Vec<Changes>> {
    let mut args = vec![
        "--git-dir",
        path,
        "log",
        "--first-parent",
        "--diff-merges=first-parent",
        "--reverse",
        "--raw",
        "--no-abbrev",
        "--format=%x00%H",
    ];
    if renames {
        args.extend(["-M50%", "-l1000"]);
    } else {
        args.push("--no-renames");
    }
    args.push(tip);
    let mut child = Command::new("git")
        .args(&args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdout(Stdio::piped())
        .spawn()?;
    let out = BufReader::new(child.stdout.take().expect("piped"));
    let mut all: Vec<Changes> = Vec::new();
    for line in out.split(b'\n') {
        let line = line?;
        if let Some(sha) = line.strip_prefix(b"\0") {
            all.push(Changes {
                commit: String::from_utf8_lossy(sha).into_owned(),
                ..Default::default()
            });
        } else if line.first() == Some(&b':') {
            let c = all.last_mut().ok_or("raw line before a commit")?;
            let mut parts = line.splitn(2, |b| *b == b'\t');
            let meta = parts.next().unwrap_or_default();
            let names = parts.next().unwrap_or_default();
            let status = meta.rsplit(|b| *b == b' ').next().unwrap_or_default();
            let names: Vec<String> = names.split(|b| *b == b'\t').map(unquote).collect();
            if status.first() == Some(&b'R') && names.len() == 2 {
                c.paths.insert(names[0].clone());
                c.paths.insert(names[1].clone());
                c.renames.insert((names[0].clone(), names[1].clone()));
            } else {
                c.paths.extend(names);
            }
        }
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("git log exited with {status}").into());
    }
    Ok(all)
}

/// git quotes unusual paths ("a\303\251.ts"); decode the octal escapes.
fn unquote(n: &[u8]) -> String {
    if n.first() != Some(&b'"') {
        return String::from_utf8_lossy(n).into_owned();
    }
    let inner = &n[1..n.len().saturating_sub(1)];
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == b'\\' && i + 1 < inner.len() {
            let e = inner[i + 1];
            if e.is_ascii_digit() && i + 3 < inner.len() + 1 {
                let oct =
                    std::str::from_utf8(&inner[i + 1..(i + 4).min(inner.len())]).unwrap_or("0");
                out.push(u8::from_str_radix(oct, 8).unwrap_or(b'?'));
                i += 4;
                continue;
            }
            out.push(match e {
                b'n' => b'\n',
                b't' => b'\t',
                other => other,
            });
            i += 2;
        } else {
            out.push(inner[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Per-commit agreement between two runs over the same chain.
pub fn agreement(a: &[Changes], b: &[Changes]) -> serde_json::Value {
    let mut path_diff = 0;
    let mut rename_diff = 0;
    let mut rename_total = (0, 0);
    let mut rename_common = 0;
    let mut examples = Vec::new();
    let n = a.len().min(b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.commit, y.commit, "the two walks visit different commits");
        rename_total.0 += x.renames.len();
        rename_total.1 += y.renames.len();
        rename_common += x.renames.intersection(&y.renames).count();
        if x.paths != y.paths {
            path_diff += 1;
            if examples.len() < 5 {
                let only_a: Vec<_> = x.paths.difference(&y.paths).take(3).collect();
                let only_b: Vec<_> = y.paths.difference(&x.paths).take(3).collect();
                examples.push(serde_json::json!({"commit": x.commit, "kind": "paths", "only_left": only_a, "only_right": only_b}));
            }
        }
        if x.renames != y.renames {
            rename_diff += 1;
            if examples.len() < 10 {
                let only_a: Vec<_> = x.renames.difference(&y.renames).take(3).collect();
                let only_b: Vec<_> = y.renames.difference(&x.renames).take(3).collect();
                examples.push(serde_json::json!({"commit": x.commit, "kind": "renames", "only_left": only_a, "only_right": only_b}));
            }
        }
    }
    serde_json::json!({
        "commits": [a.len(), b.len()],
        "compared": n,
        "commits_with_different_paths": path_diff,
        "commits_with_different_renames": rename_diff,
        "renames": [rename_total.0, rename_total.1],
        "renames_in_both": rename_common,
        "examples": examples,
    })
}
