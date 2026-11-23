//! From workspace folders to repos and scopes (C1, C2; IDX-08).
//!
//! Each folder is canonicalised, checked to be inside a git work tree, and grouped by the repo's
//! common dir, so worktrees of one repo and several folders inside one repo make one repo. A folder
//! below its work tree's top level becomes a scope prefix. Only git metadata is read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::workspace::WorkspaceFile;

/// Why a folder isn't indexed. The closed set from board S01 and spec §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    NotAGitRepository,
    NotOnThisMachine,
    BareRepository,
    Missing,
    RemoteWorkspace,
    Duplicate,
}

impl SkipReason {
    /// The words the CLI, the app and CARTOGRAPH.md show.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAGitRepository => "not a git repository",
            Self::NotOnThisMachine => "not on this machine",
            Self::BareRepository => "bare repository",
            Self::Missing => "missing",
            Self::RemoteWorkspace => "remote workspace",
            Self::Duplicate => "duplicate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Skipped {
    /// The folder as the workspace file wrote it (`path` or `uri`).
    pub folder: String,
    pub reason: SkipReason,
}

/// A folder inside a repo: only paths under `prefix` produce nodes and links.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Scope {
    /// Relative to the repo's top level, `/`-separated, no trailing slash.
    pub prefix: String,
    /// `folders[].name`, if the file gave one.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRepo {
    /// The work tree's top level, canonical. For worktrees of one repo, the main worktree when the
    /// file lists it, else the first by path.
    pub root: PathBuf,
    /// The repo's common dir, canonical: the grouping key.
    pub common_dir: PathBuf,
    /// The top level's folder name.
    pub name: String,
    /// `folders[].name` of the folder that is the whole repo, shown as "Storefront (web)".
    pub display: Option<String>,
    /// Empty means the whole repo.
    pub scopes: Vec<Scope>,
    pub shallow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Resolution {
    /// Sorted by root.
    pub repos: Vec<ResolvedRepo>,
    /// In the file's order.
    pub skipped: Vec<Skipped>,
    /// `settings["files.exclude"]` globs whose value is true.
    pub files_exclude: Vec<String>,
    /// For doctor: a literal `~` or `${env:…}` in a path, a shallow repo.
    pub warnings: Vec<String>,
}

/// One workspace folder inside a repo.
struct Member {
    /// Its work tree's top level.
    top: PathBuf,
    /// The folder, canonical.
    dir: PathBuf,
    name: Option<String>,
    main: bool,
    shallow: bool,
}

/// What `git rev-parse --show-toplevel --git-common-dir` would say about a folder.
pub(crate) struct Found {
    top: Option<PathBuf>,
    common: PathBuf,
    main_worktree: bool,
    shallow: bool,
}

/// The repo a folder is in, without the user's or the repo's config affecting the answer.
pub(crate) fn discover(dir: &Path) -> Option<Found> {
    let repo = gix::discover_opts(
        dir,
        gix::discover::upwards::Options::default(),
        gix::open::Options::isolated(),
    )
    .ok()?;
    let common = canonical(repo.common_dir())?;
    let main_worktree = canonical(repo.git_dir()).as_ref() == Some(&common);
    Some(Found {
        top: repo.workdir().and_then(canonical),
        common,
        main_worktree,
        shallow: repo.is_shallow().unwrap_or(false),
    })
}

fn canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// What makes two paths the same folder: the inode on Unix, so a case-insensitive file system or a
/// second spelling can't index one repo twice; the lowercased canonical path elsewhere.
#[cfg(unix)]
fn identity(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn identity(p: &Path) -> Option<String> {
    Some(p.to_string_lossy().to_lowercase())
}

/// A `file:` URI to a path; `None` for any other scheme, which names another machine's folder.
fn file_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut it = rest.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex: String = it.by_ref().take(2).map(char::from).collect();
            bytes.push(u8::from_str_radix(&hex, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // file:///C:/x is C:/x on Windows; the host part of file://host/x isn't supported.
    let path = match path.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_owned(),
        [b'/', ..] => path,
        _ => return None,
    };
    Some(path)
}

/// Resolves the folders of a parsed workspace file. `base` is the folder that holds the file:
/// relative paths resolve from it, not from where the command ran.
pub fn resolve_folders(base: &Path, ws: &WorkspaceFile) -> Resolution {
    let mut res = Resolution {
        files_exclude: ws.files_exclude.clone(),
        ..Resolution::default()
    };
    let skip = |res: &mut Resolution, folder: &str, reason| {
        res.skipped.push(Skipped {
            folder: folder.to_owned(),
            reason,
        })
    };
    let mut seen = BTreeSet::new();
    // The folders of each repo, by common dir.
    let mut groups: BTreeMap<PathBuf, Vec<Member>> = BTreeMap::new();
    for entry in &ws.folders {
        let label = entry.path.as_deref().or(entry.uri.as_deref()).unwrap_or("");
        if ws.remote_authority.is_some() {
            skip(&mut res, label, SkipReason::RemoteWorkspace);
            continue;
        }
        let raw = match (&entry.path, &entry.uri) {
            (Some(p), _) => p.replace('\\', "/"),
            (None, Some(u)) => match file_uri(u) {
                Some(p) => p,
                None => {
                    skip(&mut res, label, SkipReason::NotOnThisMachine);
                    continue;
                }
            },
            (None, None) => {
                skip(&mut res, label, SkipReason::Missing);
                continue;
            }
        };
        // VS Code doesn't expand these, so neither do we (C1); doctor says so.
        if raw.starts_with('~') || raw.contains("${env:") {
            res.warnings.push(format!(
                "`{raw}` is used literally: VS Code doesn't expand ~ or ${{env:…}}, so neither does Cartograph"
            ));
        }
        let p = Path::new(&raw);
        let joined = if p.is_absolute() {
            p.to_owned()
        } else {
            base.join(p)
        };
        let Some(dir) = canonical(&joined).filter(|d| d.is_dir()) else {
            skip(&mut res, label, SkipReason::Missing);
            continue;
        };
        if !identity(&dir).is_some_and(|id| seen.insert(id)) {
            skip(&mut res, label, SkipReason::Duplicate);
            continue;
        }
        let Some(found) = discover(&dir) else {
            skip(&mut res, label, SkipReason::NotAGitRepository);
            continue;
        };
        let Some(top) = found.top else {
            skip(&mut res, label, SkipReason::BareRepository);
            continue;
        };
        groups.entry(found.common).or_default().push(Member {
            top,
            dir,
            name: entry.name.clone(),
            main: found.main_worktree,
            shallow: found.shallow,
        });
    }
    for (common_dir, mut folders) in groups {
        folders.sort_by(|a, b| (!a.main, &a.top, &a.dir).cmp(&(!b.main, &b.top, &b.dir)));
        let root = folders[0].top.clone();
        let whole: Vec<_> = folders.iter().filter(|f| f.top == f.dir).collect();
        let scopes = if whole.is_empty() {
            let set: BTreeSet<Scope> = folders
                .iter()
                .map(|Member { top, dir, name, .. }| Scope {
                    prefix: dir
                        .strip_prefix(top)
                        .unwrap_or(dir)
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                    name: name.clone(),
                })
                .collect();
            set.into_iter().collect()
        } else {
            Vec::new()
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shallow = folders.iter().any(|f| f.shallow);
        if shallow {
            res.warnings.push(format!(
                "{name} is a shallow clone: history before its first commit isn't indexed"
            ));
        }
        res.repos.push(ResolvedRepo {
            display: whole.iter().find_map(|f| f.name.clone()),
            root,
            common_dir,
            name,
            scopes,
            shallow,
        });
    }
    res.repos.sort_by(|a, b| a.root.cmp(&b.root));
    res
}

/// Parses `file` and resolves its folders.
pub fn resolve(file: &Path) -> Result<Resolution, crate::DiscoveryError> {
    let ws = crate::workspace::parse_workspace(file)?;
    let base = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(resolve_folders(base, &ws))
}
