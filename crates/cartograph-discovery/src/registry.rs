//! `~/.cartograph/registry.toml`: which repo belongs to which workspace (C4; IDX-11).
//!
//! The registry is the allow-list for writes: sync acts only on repos in it (F8). Entries are keyed
//! by canonical path, with the remote URL (credentials stripped) and root commit kept so a clone that
//! moves can be matched again. Writes take an OS lock on `registry.lock`, write a temp file, fsync
//! and rename, so concurrent inits leave a valid file.

use std::path::{Path, PathBuf};

use cartograph_core::ids::is_workspace_id;
use cartograph_core::paths;
use serde::{Deserialize, Serialize};

use crate::DiscoveryError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default, rename = "workspace")]
    pub workspaces: Vec<WorkspaceEntry>,
    #[serde(default, rename = "repo")]
    pub repos: Vec<RepoEntry>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// 8 hex characters; the folder under `workspaces/`.
    pub id: String,
    pub name: String,
    /// The manifest's canonical path.
    pub manifest: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoEntry {
    /// The work tree's top level, canonical.
    pub path: PathBuf,
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_commit: Option<String>,
}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> DiscoveryError + '_ {
    move |e| DiscoveryError::Registry {
        path: path.display().to_string(),
        why: e.to_string(),
    }
}

impl Registry {
    /// Reads the registry in `data`; a missing file is an empty registry. Entries that fail
    /// validation are dropped, each with a warning.
    pub fn load(data: &Path) -> Result<(Self, Vec<String>), DiscoveryError> {
        let (reg, _, warnings) = Self::load_all(data)?;
        Ok((reg, warnings))
    }

    /// [`Registry::load`], plus the entries it ignored. [`Registry::update`] writes those back as
    /// they were: an entry is ignored on read (a path now behind a symlink, another build's
    /// workspace), never erased by the next unrelated write.
    fn load_all(data: &Path) -> Result<(Self, Self, Vec<String>), DiscoveryError> {
        let path = paths::registry_file(data);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
            Err(e) => return Err(io_err(&path)(e)),
        };
        let mut reg: Self = toml::from_str(&text).map_err(|e| DiscoveryError::Registry {
            path: path.display().to_string(),
            why: e.to_string(),
        })?;
        let mut warnings = Vec::new();
        let mut ignored = Self::default();
        let (keep, skip): (Vec<_>, Vec<_>) = std::mem::take(&mut reg.workspaces)
            .into_iter()
            .partition(|w| is_workspace_id(&w.id) && w.manifest.is_absolute());
        for w in &skip {
            warnings.push(format!(
                "registry: ignored workspace `{}`, which isn't valid",
                w.id
            ));
        }
        reg.workspaces = keep;
        ignored.workspaces = skip;
        let ids: Vec<String> = reg.workspaces.iter().map(|w| w.id.clone()).collect();
        let repos = std::mem::take(&mut reg.repos);
        for r in repos {
            let why = if !r.path.is_absolute() {
                Some("its path isn't absolute")
            } else if !ids.contains(&r.workspace) {
                Some("its workspace isn't registered")
            } else {
                // A missing path is kept: it may be a moved clone that relink can match again.
                match std::fs::canonicalize(&r.path) {
                    Ok(c) if c != r.path => {
                        Some("its path now resolves somewhere else through a symlink")
                    }
                    _ => None,
                }
            };
            match why {
                Some(why) => {
                    warnings.push(format!("registry: ignored {}: {why}", r.path.display()));
                    ignored.repos.push(r);
                }
                None => reg.repos.push(r),
            }
        }
        Ok((reg, ignored, warnings))
    }

    /// Reloads the registry under the lock, applies `f` and writes it back atomically, user-only.
    pub fn update<T>(data: &Path, f: impl FnOnce(&mut Self) -> T) -> Result<T, DiscoveryError> {
        let lock_path = data.join("registry.lock");
        paths::create_private_dir(data).map_err(io_err(data))?;
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).truncate(false).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let lock = opts.open(&lock_path).map_err(io_err(&lock_path))?;
        // The kernel releases the lock if the process dies (B2); it is dropped with `lock`.
        lock.lock().map_err(io_err(&lock_path))?;
        let (mut reg, ignored, _) = Self::load_all(data)?;
        let before: Vec<String> = reg.workspaces.iter().map(|w| w.id.clone()).collect();
        let out = f(&mut reg);
        // What load ignored goes back as it was, unless this update registered the same workspace
        // or path again, or removed the workspace the entry belongs to.
        let removed: Vec<String> = before
            .into_iter()
            .filter(|id| !reg.workspaces.iter().any(|w| &w.id == id))
            .collect();
        for w in ignored.workspaces {
            if !reg.workspaces.iter().any(|k| k.id == w.id) {
                reg.workspaces.push(w);
            }
        }
        for r in ignored.repos {
            if !removed.contains(&r.workspace) && !reg.repos.iter().any(|k| k.path == r.path) {
                reg.repos.push(r);
            }
        }
        reg.version = 1;
        let text = toml::to_string(&reg).map_err(|e| DiscoveryError::Registry {
            path: lock_path.display().to_string(),
            why: e.to_string(),
        })?;
        let path = paths::registry_file(data);
        paths::write_private(&path, text.as_bytes()).map_err(io_err(&path))?;
        Ok(out)
    }

    /// The workspace whose repo holds `path`, by the longest repo path that is a prefix of it. This is
    /// how a command run in any subfolder of any repo finds its workspace, and `mcp` runs with no path.
    pub fn workspace_for(&self, path: &Path) -> Option<&WorkspaceEntry> {
        let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let repo = self
            .repos
            .iter()
            .filter(|r| p.starts_with(&r.path))
            .max_by_key(|r| r.path.components().count())?;
        self.workspaces.iter().find(|w| w.id == repo.workspace)
    }

    /// The id of the workspace with this manifest, so running init again keeps the index.
    pub fn id_for_manifest(&self, manifest: &Path) -> Option<&str> {
        self.workspaces
            .iter()
            .find(|w| w.manifest == manifest)
            .map(|w| w.id.as_str())
    }

    /// Records a workspace and its repos, replacing what was there for that id.
    // ponytail: a repo path belongs to one workspace, the last one registered. Two workspaces sharing
    // a repo need entries keyed by (path, workspace) and a rule for which one `workspace_for` picks.
    pub fn register(&mut self, ws: WorkspaceEntry, repos: Vec<RepoEntry>) {
        self.workspaces.retain(|w| w.id != ws.id);
        self.repos
            .retain(|r| r.workspace != ws.id && !repos.iter().any(|n| n.path == r.path));
        self.workspaces.push(ws);
        self.repos.extend(repos);
        self.workspaces.sort_by(|a, b| a.id.cmp(&b.id));
        self.repos.sort_by(|a, b| a.path.cmp(&b.path));
    }

    /// Points the entry of a moved clone at its new place: the entry with the same remote URL and
    /// root commit whose path no longer exists. Returns the old path. (`doctor --fix` calls this.)
    pub fn relink(
        &mut self,
        root: &Path,
        remote: Option<&str>,
        root_commit: &str,
    ) -> Option<PathBuf> {
        let e = self.repos.iter_mut().find(|r| {
            r.root_commit.as_deref() == Some(root_commit)
                && r.remote.as_deref() == remote
                && r.path != root
                && !r.path.exists()
        })?;
        Some(std::mem::replace(&mut e.path, root.to_owned()))
    }

    /// Warnings for two registered paths that hold clones of one repo (same remote and root commit).
    pub fn duplicate_clones(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, a) in self.repos.iter().enumerate() {
            for b in &self.repos[i + 1..] {
                if a.root_commit.is_some() && a.root_commit == b.root_commit && a.remote == b.remote
                {
                    out.push(format!(
                        "{} and {} are clones of the same repo",
                        a.path.display(),
                        b.path.display()
                    ));
                }
            }
        }
        out
    }
}

/// `https://user:token@host/x` becomes `https://host/x`, so a token never reaches the registry or
/// the manifest. scp-like `git@host:x` keeps its user, which is not a secret.
pub fn strip_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let host_end = rest.find('/').unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_owned(),
    }
}

/// The remote URL (credentials stripped) and root commit of the repo at `root`, from git metadata only.
// ponytail: walks the whole first-parent chain to find the root commit, tens of ms on a big repo,
// once per init. Store it in the index's meta if doctor starts calling it on every run.
pub fn repo_identity(root: &Path) -> (Option<String>, Option<String>) {
    let Ok(repo) = gix::open_opts(root, gix::open::Options::isolated()) else {
        return (None, None);
    };
    let remote = repo
        .config_snapshot()
        .string("remote.origin.url")
        .map(|u| strip_credentials(&u.to_string()));
    let root_commit = repo.head_id().ok().and_then(|head| {
        repo.rev_walk([head.detach()])
            .first_parent_only()
            .all()
            .ok()?
            .filter_map(Result::ok)
            .last()
            .map(|info| info.id.to_string())
    });
    (remote, root_commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(id: &str) -> WorkspaceEntry {
        WorkspaceEntry {
            id: id.into(),
            name: id.into(),
            manifest: PathBuf::from(format!("/w/{id}/cartograph.yaml")),
        }
    }

    fn repo(path: &Path, ws: &str) -> RepoEntry {
        RepoEntry {
            path: path.to_owned(),
            workspace: ws.into(),
            remote: Some("https://host/r.git".into()),
            root_commit: Some("a".repeat(40)),
        }
    }

    #[test]
    fn an_update_keeps_the_entries_load_ignored() {
        let d = tempfile::tempdir().unwrap();
        let raw = Registry {
            version: 1,
            workspaces: vec![ws("0000000a"), ws("not-an-id")],
            repos: vec![repo(Path::new("relative/x"), "0000000a")],
        };
        let file = paths::registry_file(d.path());
        std::fs::write(&file, toml::to_string(&raw).unwrap()).unwrap();
        let (loaded, warnings) = Registry::load(d.path()).unwrap();
        assert_eq!(
            (loaded.workspaces.len(), loaded.repos.len(), warnings.len()),
            (1, 0, 2)
        );
        // Any init or cancel writes the registry; what load ignored must survive it.
        Registry::update(d.path(), |r| r.workspaces.push(ws("0000000b"))).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let after: Registry = toml::from_str(&text).unwrap();
        assert!(
            after.workspaces.iter().any(|w| w.id == "not-an-id"),
            "{text}"
        );
        assert!(
            after.workspaces.iter().any(|w| w.id == "0000000b"),
            "{text}"
        );
        assert!(
            after
                .repos
                .iter()
                .any(|r| r.path == Path::new("relative/x")),
            "{text}"
        );
    }

    #[test]
    fn credentials_are_stripped() {
        assert_eq!(
            strip_credentials("https://user:ghp_secret@github.com/acme/x.git"),
            "https://github.com/acme/x.git"
        );
        assert_eq!(
            strip_credentials("https://github.com/a@b"),
            "https://github.com/a@b"
        );
        assert_eq!(
            strip_credentials("git@github.com:acme/x.git"),
            "git@github.com:acme/x.git"
        );
        assert_eq!(strip_credentials("ssh://git:pw@host/x"), "ssh://host/x");
    }

    #[test]
    fn a_subfolder_of_any_repo_finds_its_workspace() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        for d in ["a/src/deep", "b", "ab"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut reg = Registry::default();
        reg.register(
            ws("0000000a"),
            vec![
                repo(&root.join("a"), "0000000a"),
                repo(&root.join("b"), "0000000a"),
            ],
        );
        reg.register(ws("0000000b"), vec![repo(&root.join("ab"), "0000000b")]);
        let id = |p: &str| reg.workspace_for(&root.join(p)).map(|w| w.id.clone());
        assert_eq!(id("a/src/deep").as_deref(), Some("0000000a"));
        assert_eq!(id("b").as_deref(), Some("0000000a"));
        // A sibling whose name starts the same isn't inside `a`.
        assert_eq!(id("ab").as_deref(), Some("0000000b"));
        assert_eq!(id(""), None);
    }

    #[test]
    fn hundreds_of_repos_read_within_5_ms() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        Registry::update(&root, |reg| {
            for w in 0..30u32 {
                let id = format!("{w:08x}");
                let repos = (0..10)
                    .map(|r| repo(&root.join(format!("w{w}/r{r}")), &id))
                    .collect();
                reg.register(ws(&id), repos);
            }
        })
        .unwrap();
        let t0 = std::time::Instant::now();
        let (reg, _) = Registry::load(&root).unwrap();
        let found = reg
            .workspace_for(&root.join("w29/r9/src"))
            .map(|w| w.id.clone());
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(reg.repos.len(), 300);
        assert_eq!(found.as_deref(), Some("0000001d"));
        eprintln!("registry of 300 repos read in {ms:.2} ms");
        // Debug builds are several times slower than release; the budget is for release.
        assert!(
            ms < if cfg!(debug_assertions) { 25.0 } else { 5.0 },
            "{ms} ms"
        );
    }

    #[test]
    fn a_moved_clone_is_relinked_and_twins_are_warned_about() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        std::fs::create_dir_all(root.join("new")).unwrap();
        let mut reg = Registry::default();
        reg.register(ws("0000000a"), vec![repo(&root.join("old"), "0000000a")]);
        let c = "a".repeat(40);
        assert_eq!(
            reg.relink(&root.join("new"), Some("https://host/x.git"), &c),
            None
        );
        assert_eq!(
            reg.relink(&root.join("new"), Some("https://host/r.git"), &c),
            Some(root.join("old"))
        );
        assert_eq!(reg.workspace_for(&root.join("new")).unwrap().id, "0000000a");
        reg.repos.push(repo(&root.join("copy"), "0000000a"));
        assert_eq!(reg.duplicate_clones().len(), 1);
    }

    #[test]
    fn twenty_concurrent_writers_leave_a_valid_registry() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let handles: Vec<_> = (0..20)
            .map(|i| {
                let data = data.clone();
                std::thread::spawn(move || {
                    let id = format!("{i:08x}");
                    Registry::update(&data, |r| r.register(ws(&id), vec![])).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let (reg, warnings) = Registry::load(&data).unwrap();
        assert_eq!(reg.workspaces.len(), 20);
        assert!(warnings.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths::registry_file(&data))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn entries_are_validated_on_read() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let text = format!(
            "version = 1\n\
             [[workspace]]\nid = \"0000000a\"\nname = \"a\"\nmanifest = \"/w/cartograph.yaml\"\n\
             [[workspace]]\nid = \"../../x\"\nname = \"x\"\nmanifest = \"/w/x.yaml\"\n\
             [[repo]]\npath = \"{0}/link\"\nworkspace = \"0000000a\"\n\
             [[repo]]\npath = \"relative\"\nworkspace = \"0000000a\"\n\
             [[repo]]\npath = \"{0}/real\"\nworkspace = \"0000000a\"\n",
            root.display()
        );
        std::fs::write(paths::registry_file(&root), text).unwrap();
        let (reg, warnings) = Registry::load(&root).unwrap();
        assert_eq!(reg.workspaces.len(), 1);
        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].path, root.join("real"));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }
}
