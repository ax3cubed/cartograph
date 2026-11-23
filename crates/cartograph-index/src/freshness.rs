//! The freshness check, the dirty flag and the background sync (IDX-15, D1, D2).
//!
//! [`freshness`] compares each repository's HEAD with its indexed commit: ref reads only (gix reads
//! the loose ref or the memory-mapped packed-refs), no object read when they are equal, no network
//! and no SQLite write. Repositories are checked in parallel; the answer is memoised for 1 s per
//! workspace so a burst of MCP reads costs one check. SRV-03 and SRV-13 call this rather than
//! reading git themselves.
//!
//! [`request_sync`] is what a read calls when something is behind: it never blocks. With the lock
//! held it leaves the dirty flag (the running sync does one more round); otherwise, with
//! `auto_sync = "background"`, it starts `cartograph sync --quiet <dir>` detached, at most once per
//! workspace per 2 s.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use cartograph_git::{ObjectId, Repo};
use cartograph_store::FORMAT_VERSION;
use cartograph_store::rusqlite::{Connection, OpenFlags, OptionalExtension};
use rayon::prelude::*;

use cartograph_store::writer::{WriterLock, mark_dirty};

use crate::sync::{SyncError, Workspace};

/// Commits counted behind before reporting "1000+" (proposed).
pub const BEHIND_CAP: u32 = 1_000;
const BEHIND_BUDGET: Duration = Duration::from_millis(20);
const MEMO: Duration = Duration::from_secs(1);
const SPAWN_EVERY: Duration = Duration::from_secs(2);

/// The workspace's state, as spec §7 names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No index for this workspace.
    None,
    Current,
    Behind,
    /// The writer holds the lock.
    Syncing,
    /// An indexed commit is gone from its branch.
    Rewritten,
    /// The index format is older than this binary.
    NeedsMigrate,
    /// The last run stopped part way through, or the index can't be read.
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Current => "current",
            Self::Behind => "behind",
            Self::Syncing => "syncing",
            Self::Rewritten => "rewritten",
            Self::NeedsMigrate => "needs-migrate",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFreshness {
    pub name: String,
    pub indexed: Option<ObjectId>,
    /// The indexed branch's tip (HEAD when the repo has no branch set).
    pub head: Option<ObjectId>,
    /// First-parent commits from the indexed commit to HEAD, at most [`BEHIND_CAP`].
    pub behind: u32,
    /// The count stopped at the cap or the 20 ms budget: show it as "1000+".
    pub behind_capped: bool,
    pub rewritten: bool,
    pub shallow: bool,
    /// HEAD is detached or on another branch than the indexed one (spec §8).
    pub on_branch_other_than_indexed: bool,
    /// Uncommitted changes; `None` here ("not checked yet", which the spec allows), since this check
    /// runs on every read under a 100 ms budget. The app's repo list fills it with [`uncommitted`].
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshnessReport {
    pub state: State,
    /// The writer lock is held.
    pub syncing: bool,
    /// A sync was requested while one was running (D2).
    pub dirty_flag: bool,
    pub repos: Vec<RepoFreshness>,
}

impl RepoFreshness {
    /// Has a tip that was never indexed. A repo whose branch doesn't resolve (no commits yet, or no
    /// such local branch) has nothing a sync could index, so it isn't behind: counting it would
    /// start a sync that can never catch up, again and again.
    pub fn unindexed(&self) -> bool {
        self.indexed.is_none() && self.head.is_some()
    }
}

impl FreshnessReport {
    /// Any repo behind, rewritten or never indexed: the same repos that make the state behind.
    pub fn behind(&self) -> bool {
        self.repos
            .iter()
            .any(|r| r.behind > 0 || r.rewritten || r.unindexed())
    }
}

static MEMOS: LazyLock<Mutex<HashMap<PathBuf, (Instant, FreshnessReport)>>> =
    LazyLock::new(Default::default);
static SPAWNS: LazyLock<Mutex<HashMap<PathBuf, Instant>>> = LazyLock::new(Default::default);

/// [`check`], memoised for 1 s per workspace.
pub fn freshness(ws: &Workspace) -> FreshnessReport {
    let mut memo = MEMOS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, r)) = memo.get(&ws.dir)
        && at.elapsed() < MEMO
    {
        return r.clone();
    }
    let r = check(ws);
    memo.insert(ws.dir.clone(), (Instant::now(), r.clone()));
    r
}

/// What an index says about itself, read without a write and without `quick_check`.
struct Indexed {
    format: i64,
    min_reader: i64,
    complete: bool,
    heads: HashMap<String, Vec<u8>>,
}

fn read_indexed(path: &Path) -> Option<Indexed> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let meta = |k: &str| {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| {
            r.get::<_, cartograph_store::rusqlite::types::Value>(0)
        })
        .optional()
        .ok()
        .flatten()
    };
    use cartograph_store::rusqlite::types::Value;
    let int = |v: Option<Value>| match v {
        Some(Value::Integer(i)) => Some(i),
        _ => None,
    };
    let format = int(meta("format_version"))?;
    let min_reader = int(meta("min_reader_version")).unwrap_or(format);
    let complete = matches!(meta("state"), Some(Value::Text(s)) if s == "complete");
    let heads = conn
        .prepare("SELECT name, head FROM repo WHERE head IS NOT NULL")
        .ok()?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;
    Some(Indexed {
        format,
        min_reader,
        complete,
        heads,
    })
}

/// Whether a writer holds the lock, probed with a shared lock on a read-only handle, so the read
/// path writes nothing.
///
/// ponytail: the probe holds the lock for a moment; a sync starting in that moment sees it busy,
/// sets the dirty flag and exits. The next read asks again (D1), so the request is late, not lost
/// for good. A pid check would close it but needs a platform call.
fn lock_held(ws: &Workspace) -> bool {
    let Ok(f) = std::fs::File::open(ws.lock_path()) else {
        return false;
    };
    matches!(f.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock))
}

/// The freshness of every repository of `ws`, uncached.
pub fn check(ws: &Workspace) -> FreshnessReport {
    let syncing = lock_held(ws);
    let dirty_flag = ws.dirty_path().exists();
    let path = ws.index_path();
    let report = |state, repos| FreshnessReport {
        state,
        syncing,
        dirty_flag,
        repos,
    };
    if !path.exists() {
        return report(
            if syncing { State::Syncing } else { State::None },
            Vec::new(),
        );
    }
    let Some(ix) = read_indexed(&path) else {
        return report(State::Failed, Vec::new());
    };
    if ix.format > FORMAT_VERSION && ix.min_reader > FORMAT_VERSION {
        return report(State::Failed, Vec::new());
    }
    let repos: Vec<RepoFreshness> = ws
        .repos
        .par_iter()
        .map(|r| {
            let indexed = ix
                .heads
                .get(&r.name)
                .and_then(|b| ObjectId::try_from(b.as_slice()).ok());
            repo_freshness(&r.name, &r.path, r.branch.as_deref(), indexed)
        })
        .collect();
    let state = if syncing {
        State::Syncing
    } else if ix.format < FORMAT_VERSION {
        State::NeedsMigrate
    } else if !ix.complete {
        State::Failed
    } else if repos.iter().any(|r| r.rewritten) {
        State::Rewritten
    } else if repos.iter().any(|r| r.behind > 0 || r.unindexed()) {
        State::Behind
    } else {
        State::Current
    };
    report(state, repos)
}

/// The tip of `branch`: the local branch, else origin's remote-tracking one. The default-branch
/// ladder (IDX-09) takes origin/HEAD's branch even when it was never checked out (`git clone -b
/// develop` leaves only origin/main), so sync and the freshness check walk that commit, not none.
pub(crate) fn branch_tip(repo: &Repo, branch: &str) -> Result<ObjectId, cartograph_git::GitError> {
    repo.resolve(&format!("refs/heads/{branch}"))
        .or_else(|_| repo.resolve(&format!("refs/remotes/origin/{branch}")))
}

/// One repository: the indexed branch's tip (HEAD when `branch` is `None`) against the indexed
/// commit, then a capped first-parent count.
pub fn repo_freshness(
    name: &str,
    path: &Path,
    branch: Option<&str>,
    indexed: Option<ObjectId>,
) -> RepoFreshness {
    let mut out = RepoFreshness {
        name: name.to_owned(),
        indexed,
        head: None,
        behind: 0,
        behind_capped: false,
        rewritten: false,
        shallow: false,
        on_branch_other_than_indexed: false,
        dirty: None,
    };
    let Ok(repo) = Repo::open(path) else {
        return out;
    };
    out.shallow = repo.is_shallow();
    if let Some(b) = branch {
        out.on_branch_other_than_indexed = repo
            .local()
            .head_name()
            .ok()
            .flatten()
            .is_none_or(|n| n.shorten() != b);
    }
    let tip = match branch {
        Some(b) => branch_tip(&repo, b),
        None => repo.head(),
    };
    let Ok(head) = tip else {
        return out;
    };
    out.head = Some(head);
    let Some(indexed) = indexed else {
        return out;
    };
    if indexed == head {
        return out;
    }
    let started = Instant::now();
    let local = repo.local();
    let Ok(walk) = local.rev_walk([head]).first_parent_only().all() else {
        return out;
    };
    for info in walk {
        let Ok(info) = info else {
            out.rewritten = true;
            return out;
        };
        if info.id == indexed {
            return out;
        }
        if out.behind == BEHIND_CAP || started.elapsed() > BEHIND_BUDGET {
            out.behind_capped = true;
            // Over the cap: rewritten only if the indexed commit isn't an ancestor at all.
            out.rewritten = !local
                .merge_base(indexed, head)
                .is_ok_and(|b| b.is_some_and(|b| b.detach() == indexed));
            return out;
        }
        out.behind += 1;
    }
    // The whole chain was walked without meeting the indexed commit.
    out.rewritten = true;
    out
}

/// Whether the work tree at `path` has uncommitted changes to tracked files; `None` when git can't
/// say. Untracked files don't count.
///
/// ponytail: one `git status` per call (about 10 ms on a small repo, more on a large one), so it is
/// for the repo list, not the freshness check. The ceiling is a large repo on every list; then gix's
/// `status` feature, cached by the mtime of `.git/index`.
pub fn uncommitted(path: &Path) -> Option<bool> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        // No index refresh written into the user's repo, and no fsmonitor hook run.
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(!out.stdout.is_empty())
}

/// `auto_sync` in config (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSync {
    Background,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRequest {
    /// A sync holds the lock; the dirty flag is set and it will run one more round.
    AlreadyRunning,
    /// A detached `cartograph sync` was started.
    Spawned,
    /// `auto_sync = "never"`.
    Disabled,
    /// A sync was started for this workspace less than 2 s ago.
    RateLimited,
}

/// Asks for a sync without waiting for it. `dir` is passed to the child as its `[dir]`.
pub fn request_sync(ws: &Workspace, auto: AutoSync, dir: &Path) -> Result<SyncRequest, SyncError> {
    let exe = std::env::current_exe().map_err(|source| SyncError::Io {
        path: "the cartograph binary".into(),
        source,
    })?;
    request_sync_with(ws, auto, &exe, dir)
}

/// [`request_sync`] with the binary given, for tests. The product always passes `current_exe()`,
/// never a name looked up on PATH.
pub fn request_sync_with(
    ws: &Workspace,
    auto: AutoSync,
    exe: &Path,
    dir: &Path,
) -> Result<SyncRequest, SyncError> {
    if auto == AutoSync::Never {
        return Ok(SyncRequest::Disabled);
    }
    crate::sync::create_dir(ws)?;
    let io = |source| SyncError::Io {
        path: ws.dir.display().to_string(),
        source,
    };
    // The flag first, as in `sync`, so a sync finishing right now still sees it.
    mark_dirty(&ws.dir).map_err(io)?;
    match WriterLock::try_acquire(&ws.dir).map_err(io)? {
        None => return Ok(SyncRequest::AlreadyRunning),
        Some(l) => drop(l),
    }
    {
        let mut spawns = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
        if spawns
            .get(&ws.dir)
            .is_some_and(|at| at.elapsed() < SPAWN_EVERY)
        {
            return Ok(SyncRequest::RateLimited);
        }
        spawns.insert(ws.dir.clone(), Instant::now());
    }
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
    let mut cmd = Command::new(exe);
    cmd.arg("sync")
        .arg("--quiet")
        .arg(&dir)
        .current_dir(&ws.dir)
        .env_clear()
        .envs(child_env(std::env::vars_os()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own process group, so a Ctrl-C in the terminal that ran the read doesn't stop it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|source| SyncError::Io {
        path: exe.display().to_string(),
        source,
    })?;
    // Reaped when it ends, so a long-running server collects no zombies; a CLI exits without
    // waiting and the child carries on.
    std::thread::spawn(move || child.wait());
    Ok(SyncRequest::Spawned)
}

/// The environment a background child keeps: PATH, HOME, the XDG folders and CARTOGRAPH_HOME (it
/// moves the data folder, so the child must see it), and on Windows USERPROFILE (where the data
/// folder is when HOME isn't set, as it usually isn't), SystemRoot, TEMP and TMP. Windows spells
/// names in any case (`Path`). Everything else, tokens included, is dropped.
fn child_env(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    vars.filter(|(k, _)| {
        let k = k.to_string_lossy().to_ascii_uppercase();
        matches!(
            k.as_str(),
            "PATH" | "HOME" | "CARTOGRAPH_HOME" | "USERPROFILE" | "SYSTEMROOT" | "TEMP" | "TMP"
        ) || k.starts_with("XDG_")
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::child_env;

    #[test]
    fn a_background_child_keeps_only_path_home_and_xdg() {
        let vars = [
            ("PATH", "/bin"),
            ("HOME", "/home/a"),
            ("XDG_DATA_HOME", "/x"),
            ("CARTOGRAPH_HOME", "/c"),
            ("GITHUB_TOKEN", "secret"),
            ("ANTHROPIC_API_KEY", "secret"),
            ("SSH_AUTH_SOCK", "/tmp/s"),
        ]
        .map(|(k, v)| (k.into(), v.into()));
        let mut kept: Vec<String> = child_env(vars.into_iter())
            .into_iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept, ["CARTOGRAPH_HOME", "HOME", "PATH", "XDG_DATA_HOME"]);
    }

    #[test]
    fn a_windows_child_keeps_what_finds_its_data_folder() {
        // cmd and PowerShell set no HOME: the data folder comes from USERPROFILE.
        let vars = [
            ("Path", "C:\\Windows"),
            ("USERPROFILE", "C:\\Users\\a"),
            ("SystemRoot", "C:\\Windows"),
            ("TEMP", "C:\\t"),
            ("TMP", "C:\\t"),
            ("GH_TOKEN", "secret"),
        ]
        .map(|(k, v)| (k.into(), v.into()));
        let mut kept: Vec<String> = child_env(vars.into_iter())
            .into_iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept, ["Path", "SystemRoot", "TEMP", "TMP", "USERPROFILE"]);
    }
}
