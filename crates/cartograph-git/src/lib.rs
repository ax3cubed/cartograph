//! Git reading with gitoxide (IDX-04, B7): the first-parent walk with a tree diff per commit, blob
//! reads with a size cap, and the tree at a commit.
//!
//! A repository is untrusted content. It is opened isolated (no hooks, no global or system config,
//! so no filter drivers, external diff or credential helpers), and only the object database is read,
//! never the checkout. Tree paths are opaque strings and are never joined to a real directory.
//!
//! ponytail: one concrete `Repo`, no `ObjectSource` trait yet. The trait arrives with its second
//! implementation, the git-command fallback for partial clones (B7).

use std::collections::BTreeMap;
use std::path::Path;

use cartograph_core::Explain;
use gix::bstr::ByteSlice;
use gix::diff::tree::recorder::Location;
use gix::diff::tree_with_rewrites::{ChangeRef, Options};
use gix::objs::TreeRefIter;

pub use gix::ObjectId;

#[doc(hidden)]
pub mod fixture;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("can't open the git repository at {path}")]
    Open {
        path: String,
        #[source]
        source: Box<gix::Error>,
    },
    #[error("git object error")]
    Object(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("{from} is not on the first-parent history of {to}")]
    NotAncestor { from: ObjectId, to: ObjectId },
    #[error("blob {oid} is {size} bytes, over the {max} byte cap")]
    TooLarge { oid: ObjectId, size: u64, max: u64 },
}

impl Explain for GitError {
    fn why(&self) -> String {
        match self {
            Self::Open { source, .. } => source.to_string(),
            Self::Object(e) => e.to_string(),
            Self::NotAncestor { .. } => "the history was rewritten since it was indexed".into(),
            Self::TooLarge { .. } => "large blobs are skipped to bound memory".into(),
        }
    }
    fn action(&self) -> String {
        match self {
            Self::Open { .. } => "pass the folder of a git repository".into(),
            Self::NotAncestor { .. } => "cartograph index --rebuild".into(),
            _ => "run git fsck on the repository".into(),
        }
    }
}

fn obj<E: std::error::Error + Send + Sync + 'static>(e: E) -> GitError {
    GitError::Object(Box::new(e))
}

/// One changed regular file. `None` means absent: deleted, or a symlink or gitlink (as in the research).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
}

/// One commit on the first-parent chain, diffed against its first parent only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitStep {
    pub sha: ObjectId,
    /// The committer time, in seconds.
    pub time: i64,
    /// `max(time, previous corrected time + 1)`: strictly increasing along the chain (bench.mjs).
    pub corrected_time: i64,
    pub author: String,
    pub summary: String,
    /// Sorted by path.
    pub changes: Vec<Change>,
}

/// A repository opened for reading. Cheap to share across threads; call [`Repo::local`] per worker.
pub struct Repo {
    inner: gix::ThreadSafeRepository,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self, GitError> {
        let inner =
            gix::open_opts(path, gix::open::Options::isolated()).map_err(|e| GitError::Open {
                path: path.display().to_string(),
                source: Box::new(e),
            })?;
        Ok(Self {
            inner: inner.into_sync(),
        })
    }

    /// A handle for the current thread, with an object cache.
    pub fn local(&self) -> gix::Repository {
        let mut r = self.inner.to_thread_local();
        r.object_cache_size_if_unset(16 * 1024 * 1024);
        r
    }

    pub fn head(&self) -> Result<ObjectId, GitError> {
        self.resolve("HEAD")
    }

    pub fn resolve(&self, rev: &str) -> Result<ObjectId, GitError> {
        Ok(self
            .local()
            .rev_parse_single(rev)
            .map_err(obj)?
            .object()
            .map_err(obj)?
            .peel_to_commit()
            .map_err(obj)?
            .id)
    }

    pub fn is_shallow(&self) -> bool {
        self.local().is_shallow().unwrap_or(false)
    }

    pub fn is_bare(&self) -> bool {
        self.local().is_bare()
    }

    /// The first-parent commits after `from` (exclusive) up to `to`, oldest first, each with the regular
    /// files it changed that pass `keep`. With `from = None` the walk starts at the root commit.
    ///
    /// ponytail: `keep` runs inside the diff callback, but subtrees are still descended; pruning them
    /// needs the visitor-level diff API. A commit with a huge number of changed paths is held whole.
    pub fn walk_first_parent<F: Fn(&str) -> bool>(
        &self,
        from: Option<ObjectId>,
        to: ObjectId,
        keep: F,
    ) -> Result<Walk<F>, GitError> {
        let repo = self.local();
        let mut chain = Vec::new();
        let mut found = from.is_none();
        for info in repo.rev_walk([to]).first_parent_only().all().map_err(obj)? {
            let id = info.map_err(obj)?.id;
            if Some(id) == from {
                found = true;
                break;
            }
            chain.push(id);
        }
        if !found {
            return Err(GitError::NotAncestor {
                from: from.expect("checked"),
                to,
            });
        }
        chain.reverse();
        let (prev_tree, prev_corrected) = match from {
            Some(f) => {
                let c = repo.find_commit(f).map_err(obj)?;
                (
                    Some(c.tree_id().map_err(obj)?.detach()),
                    c.time().map_err(obj)?.seconds,
                )
            }
            None => (None, i64::MIN),
        };
        let cache = repo.diff_resource_cache_for_tree_diff().map_err(obj)?;
        Ok(Walk {
            repo,
            chain: chain.into_iter(),
            prev_tree,
            prev_corrected,
            cache,
            state: Default::default(),
            keep,
        })
    }

    /// Every regular file at `commit` that passes `keep`: the replay's oracle (`git ls-tree -r`).
    pub fn tree_at(
        &self,
        commit: ObjectId,
        keep: impl Fn(&str) -> bool,
    ) -> Result<BTreeMap<String, ObjectId>, GitError> {
        let repo = self.local();
        let tree = repo.find_commit(commit).map_err(obj)?.tree().map_err(obj)?;
        let mut rec = gix::traverse::tree::Recorder::default();
        tree.traverse().breadthfirst(&mut rec).map_err(obj)?;
        Ok(rec
            .records
            .into_iter()
            .filter(|e| e.mode.is_blob())
            .map(|e| (e.filepath.to_str_lossy().into_owned(), e.oid))
            .filter(|(p, _)| keep(p))
            .collect())
    }
}

/// Reads a blob, checking its size from the header first so an oversized body is never read.
pub fn read_blob(repo: &gix::Repository, oid: ObjectId, max: u64) -> Result<Vec<u8>, GitError> {
    let size = repo.find_header(oid).map_err(obj)?.size();
    if size > max {
        return Err(GitError::TooLarge { oid, size, max });
    }
    Ok(repo.find_blob(oid).map_err(obj)?.take_data())
}

/// The iterator [`Repo::walk_first_parent`] returns.
pub struct Walk<F> {
    repo: gix::Repository,
    chain: std::vec::IntoIter<ObjectId>,
    prev_tree: Option<ObjectId>,
    prev_corrected: i64,
    cache: gix::diff::blob::Platform,
    state: gix::diff::tree::State,
    keep: F,
}

impl<F> Walk<F> {
    /// Continues the corrected times of an earlier walk that ended at `from`.
    pub fn with_prev_corrected(mut self, t: i64) -> Self {
        self.prev_corrected = t;
        self
    }

    /// Commits left to walk.
    pub fn remaining(&self) -> usize {
        self.chain.len()
    }
}

impl<F: Fn(&str) -> bool> Walk<F> {
    fn step(&mut self, id: ObjectId) -> Result<CommitStep, GitError> {
        let commit = self.repo.find_commit(id).map_err(obj)?;
        let tree_id = commit.tree_id().map_err(obj)?.detach();
        let time = commit.time().map_err(obj)?.seconds;
        let author = commit
            .author()
            .map_err(obj)?
            .name
            .to_str_lossy()
            .into_owned();
        let summary = commit
            .message()
            .map_err(obj)?
            .summary()
            .to_str_lossy()
            .into_owned();
        let new = self.repo.find_tree(tree_id).map_err(obj)?;
        let old = match self.prev_tree {
            Some(p) => self.repo.find_tree(p).map_err(obj)?,
            None => self.repo.empty_tree(),
        };
        let mut changes = Vec::new();
        let keep = &self.keep;
        let regular = |m: gix::object::tree::EntryMode, id: ObjectId| m.is_blob().then_some(id);
        self.cache.clear_resource_cache_keep_allocation();
        gix::diff::tree_with_rewrites(
            TreeRefIter::from_bytes(&old.data, old.id.kind()),
            TreeRefIter::from_bytes(&new.data, new.id.kind()),
            &mut self.cache,
            &mut self.state,
            &self.repo.objects,
            |change| -> Result<_, gix::Error> {
                let (location, old, new) = match change {
                    ChangeRef::Addition {
                        location,
                        entry_mode,
                        id,
                        ..
                    } => (location, None, regular(entry_mode, id)),
                    ChangeRef::Deletion {
                        location,
                        entry_mode,
                        id,
                        ..
                    } => (location, regular(entry_mode, id), None),
                    ChangeRef::Modification {
                        location,
                        previous_entry_mode,
                        previous_id,
                        entry_mode,
                        id,
                        ..
                    } => (
                        location,
                        regular(previous_entry_mode, previous_id),
                        regular(entry_mode, id),
                    ),
                    ChangeRef::Rewrite { .. } => unreachable!("rewrites are off"),
                };
                if old != new {
                    let path = location.to_str_lossy();
                    if keep(&path) {
                        changes.push(Change {
                            path: path.into_owned(),
                            old,
                            new,
                        });
                    }
                }
                Ok(std::ops::ControlFlow::Continue(()))
            },
            Options {
                location: Some(Location::Path),
                rewrites: None,
            },
        )
        .map_err(obj)?;
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let corrected_time = time.max(self.prev_corrected.saturating_add(1));
        self.prev_corrected = corrected_time;
        self.prev_tree = Some(tree_id);
        Ok(CommitStep {
            sha: id,
            time,
            corrected_time,
            author,
            summary,
            changes,
        })
    }
}

impl<F: Fn(&str) -> bool> Iterator for Walk<F> {
    type Item = Result<CommitStep, GitError>;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.chain.next()?;
        Some(self.step(id))
    }
}

#[cfg(test)]
mod tests;
