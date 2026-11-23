//! The workspace, repo and freshness reads: what the index stored, nothing computed from git.
//!
//! ponytail: `behind` is 0 and `branch` empty, because neither is stored yet. SRV-03 reads each
//! repo's HEAD after the answer and caches it (D1); the indexed branch arrives with the registry
//! (IDX-11).

use cartograph_api::types::{Freshness, IndexState, Repo, RepoBehind, WorkspaceSummary};
use cartograph_core::RepoId;
use cartograph_store::State;
use cartograph_store::rusqlite::OptionalExtension;

use crate::resolve::hex;
use crate::{QueryError, Snapshot, rfc3339};

struct RepoRow {
    id: i64,
    name: String,
    path: String,
    head: Option<Vec<u8>>,
}

impl Snapshot<'_> {
    /// The index's own state as the wire spells it.
    pub fn index_state(&self) -> Result<IndexState, QueryError> {
        if self.store_state() == State::NeedsMigrate {
            return Ok(IndexState::NeedsMigrate);
        }
        let s: Option<String> = self
            .conn()
            .prepare_cached("SELECT CAST(value AS TEXT) FROM meta WHERE key = 'state'")?
            .query_row([], |r| r.get(0))
            .optional()?;
        Ok(match s.as_deref() {
            Some("complete") => IndexState::Current,
            // A mid-sync index is readable: batches commit whole (IDX-13).
            Some("building" | "syncing") => IndexState::Syncing,
            _ => IndexState::Failed,
        })
    }

    fn built_at(&self) -> Result<Option<String>, QueryError> {
        let t: Option<i64> = self
            .conn()
            .prepare_cached("SELECT max(finished_at) FROM run")?
            .query_row([], |r| r.get(0))?;
        Ok(t.map(rfc3339))
    }

    fn count(&self, sql: &str) -> Result<u32, QueryError> {
        let n: i64 = self
            .conn()
            .prepare_cached(sql)?
            .query_row([], |r| r.get(0))?;
        Ok(n as u32)
    }

    /// The workspace's repos now. A repo dropped from the workspace keeps its row and history for
    /// time travel (`removed = 1`, IDX-14), but isn't listed or counted.
    fn repo_rows(&self) -> Result<Vec<RepoRow>, QueryError> {
        let mut st = self.conn().prepare_cached(
            "SELECT id, name, path, head FROM repo WHERE removed = 0 ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(RepoRow {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                head: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn workspace_summary(&self, id: &str) -> Result<WorkspaceSummary, QueryError> {
        let repos = self.repo_rows()?;
        let first = repos.first();
        Ok(WorkspaceSummary {
            id: id.into(),
            name: first.map_or_else(|| id.to_owned(), |r| r.name.clone()),
            path: first.map(|r| r.path.clone()).unwrap_or_default(),
            demo: false,
            repos: repos.len() as u32,
            services: 0,
            modules: 0,
            files: self.count("SELECT count(*) FROM file_path WHERE to_seq IS NULL")?,
            commits: self.count("SELECT count(*) FROM commits")?,
            state: self.index_state()?,
            built_at: self.built_at()?,
        })
    }

    pub fn repos(&self) -> Result<Vec<Repo>, QueryError> {
        // Each repo's own steps: the workspace total is the sum of these.
        let mut count = self
            .conn()
            .prepare_cached("SELECT count(*) FROM commits WHERE repo = ?1")?;
        let mut out = Vec::new();
        for r in self.repo_rows()? {
            let commits: i64 = count.query_row([r.id], |x| x.get(0))?;
            let head = r.head.as_deref().map(hex);
            out.push(Repo {
                // ponytail: the repo's name stands in for its registry key until IDX-11.
                id: RepoId(r.name.clone()),
                name: r.name,
                path: r.path,
                branch: String::new(),
                indexed_commit: head.clone(),
                head: head.unwrap_or_default(),
                behind: 0,
                behind_capped: false,
                dirty: None,
                commits: commits as u32,
            });
        }
        Ok(out)
    }

    /// Why the last run didn't finish, as the writer recorded it on resuming (`meta.last_error`).
    pub fn last_error(&self) -> Result<Option<String>, QueryError> {
        Ok(self
            .conn()
            .prepare_cached("SELECT CAST(value AS TEXT) FROM meta WHERE key = 'last_error'")?
            .query_row([], |r| r.get(0))
            .optional()?)
    }

    /// The stored state: no git call, so it answers in microseconds (D1).
    pub fn freshness(&self) -> Result<Freshness, QueryError> {
        let format_version: i64 = self
            .conn()
            .prepare_cached("SELECT value FROM meta WHERE key = 'format_version'")?
            .query_row([], |r| r.get(0))?;
        Ok(Freshness {
            state: self.index_state()?,
            format_version: format_version as u32,
            built_at: self.built_at()?,
            behind: 0,
            repos: self
                .repo_rows()?
                .into_iter()
                .map(|r| RepoBehind {
                    repo: RepoId(r.name.clone()),
                    name: r.name,
                    behind: 0,
                    behind_capped: false,
                })
                .collect(),
            progress: None,
            error: None,
        })
    }
}
