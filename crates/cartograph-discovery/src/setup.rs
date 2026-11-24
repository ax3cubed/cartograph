//! What `cartograph init` does before it indexes: find the workspace, resolve it, pick each repo's
//! branch, then write the manifest and record the workspace in the registry.
//!
//! Split in two so the CLI can show the plan and ask before anything is written: [`plan`] only
//! reads; [`apply`] writes `cartograph.yaml`, the data folder and the registry.

use std::path::{Path, PathBuf};

use cartograph_core::{ids, paths};

use crate::manifest::{self, Manifest, Options, Repo, Workspace};
use crate::registry::{Registry, RepoEntry, WorkspaceEntry, repo_identity};
use crate::resolve::{Resolution, resolve_folders};
use crate::workspace::{find_workspace_files, folders_without_workspace_file, parse_workspace};
use crate::{DiscoveryError, Source, branch};

/// One repo as init will record it.
#[derive(Debug, Clone)]
pub struct PlannedRepo {
    pub root: PathBuf,
    pub branch: String,
    pub source: Source,
    pub remote: Option<String>,
    pub root_commit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// The `.code-workspace` file, or `None` when the folders came from a walk (C2).
    pub workspace_file: Option<PathBuf>,
    /// Where `cartograph.yaml` goes: beside the workspace file (C5). Canonical.
    pub manifest_path: PathBuf,
    pub resolution: Resolution,
    /// One per repo of `resolution.repos`, in its order. Not parallel to `manifest.repos`, which keeps
    /// the committed order and entries not cloned here: pair the two by folder (then name).
    pub repos: Vec<PlannedRepo>,
    pub manifest: Manifest,
    /// Doctor-level notes: literal `~`, shallow clones, current-branch fallbacks, unknown keys.
    pub warnings: Vec<String>,
}

/// Finds the workspace for `dir` (or uses `workspace_file`), resolves it and picks each branch. An
/// existing `cartograph.yaml` there supplies `default_branch` per repo. Reads only.
pub fn plan(dir: &Path, workspace_file: Option<&Path>) -> Result<Plan, DiscoveryError> {
    let file = match workspace_file {
        Some(f) => Some(f.to_owned()),
        None => {
            let mut found = find_workspace_files(dir);
            match found.len() {
                0 => None,
                1 => found.pop(),
                _ => {
                    return Err(DiscoveryError::ManyWorkspaces {
                        dir: dir.display().to_string(),
                        files: found.iter().map(|f| f.display().to_string()).collect(),
                    });
                }
            }
        }
    };
    let (base, ws) = match &file {
        Some(f) => (
            f.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_owned(),
            parse_workspace(f)?,
        ),
        None => (
            dir.to_owned(),
            crate::workspace::WorkspaceFile {
                folders: folders_without_workspace_file(dir),
                ..Default::default()
            },
        ),
    };
    let resolution = resolve_folders(&base, &ws);
    if file.is_none() && resolution.repos.is_empty() {
        return Err(DiscoveryError::NoWorkspace {
            dir: dir.display().to_string(),
        });
    }
    let base = std::fs::canonicalize(&base).map_err(|e| DiscoveryError::WorkspaceFile {
        path: base.display().to_string(),
        why: e.to_string(),
    })?;
    let manifest_path = base.join(manifest::FILE_NAME);
    let mut warnings = resolution.warnings.clone();
    let existing = if manifest_path.is_file() {
        let p = manifest::read(&manifest_path)?;
        warnings.extend(p.warnings);
        Some(p.manifest)
    } else {
        None
    };

    // The committed manifest is the base (C5): its repos stay as written, whether or not they are
    // cloned on this machine, and so do its excludes, services and rules. A discovered repo matches
    // a committed one by folder, then by name; only one that matches none is added.
    let mut previous = existing.unwrap_or_else(|| Manifest {
        version: manifest::VERSION,
        workspace: Workspace {
            name: file
                .as_ref()
                .and_then(|f| f.file_stem())
                .or_else(|| base.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".into()),
            file: None,
        },
        repos: vec![],
        options: Options::default(),
        services: vec![],
        rules: vec![],
        baseline: None,
        retrieval: Default::default(),
    });
    let committed_at: Vec<Option<PathBuf>> = previous
        .repos
        .iter()
        .map(|e| std::fs::canonicalize(manifest::repo_dir(&base, e)).ok())
        .collect();
    let committed = previous.repos.len();
    let mut repos = Vec::new();
    for r in &resolution.repos {
        let by_folder = committed_at
            .iter()
            .position(|p| p.as_deref() == Some(r.root.as_path()));
        // By name only when the committed entry's folder isn't here: it is the same repo, cloned
        // somewhere else on this machine.
        let by_name = || {
            let n = repo_name(&r.name);
            (0..committed).find(|i| committed_at[*i].is_none() && previous.repos[*i].name == n)
        };
        let matched = by_folder.or_else(by_name);
        let pinned = matched.and_then(|i| previous.repos[i].default_branch.clone());
        let (branch, source) = branch::default_branch(&r.root, pinned.as_deref())?;
        let (remote, root_commit) = repo_identity(&r.root);
        let name = matched.map(|i| previous.repos[i].name.clone());
        if source == Source::CurrentBranch {
            warnings.push(format!(
                "{}: no origin/HEAD, main or master, so the current branch `{branch}` is indexed; \
                 set default_branch in cartograph.yaml to choose",
                name.as_deref().unwrap_or(&r.name)
            ));
        }
        if name.is_none() {
            // Two folders with one name (absolute paths elsewhere) get a suffix, so names stay keys.
            let base_name = repo_name(&r.name);
            let mut name = base_name.clone();
            let mut n = 2;
            while previous.repos.iter().any(|e| e.name == name) {
                name = format!("{base_name}-{n}");
                n += 1;
            }
            previous.repos.push(Repo {
                name,
                path: manifest::relative_path(&base, &r.root),
                display: r.display.clone(),
                remote: remote.clone(),
                default_branch: Some(branch.clone()),
                scopes: r.scopes.iter().map(|s| s.prefix.clone()).collect(),
            });
        }
        repos.push(PlannedRepo {
            root: r.root.clone(),
            branch,
            source,
            remote,
            root_commit,
        });
    }
    // Excludes found in the workspace file are added to the committed ones, never in place of them.
    for e in &resolution.files_exclude {
        if !previous.options.exclude.contains(e) {
            previous.options.exclude.push(e.clone());
        }
    }
    if previous.workspace.file.is_none() {
        previous.workspace.file = file
            .as_ref()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().into_owned());
    }
    let manifest = Manifest {
        version: manifest::VERSION,
        ..previous
    };
    Ok(Plan {
        workspace_file: file,
        manifest_path,
        resolution,
        repos,
        manifest,
        warnings,
    })
}

/// A repo name usable as a path prefix in the merged tree (the indexer's `valid_repo_name`): ASCII
/// letters, digits, `.`, `_` and `-`, not hidden, at most 60 characters, and not a folder name the
/// extractors skip. The folder name is kept when it already fits; `display` keeps the original.
pub fn repo_name(folder: &str) -> String {
    let mut n: String = folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_start_matches('.')
        .chars()
        .take(60)
        .collect();
    if n.is_empty() {
        n.push_str("repo");
    } else if matches!(n.as_str(), "node_modules" | "dist" | "build" | "vendor") {
        n.push_str("-repo");
    }
    n
}

/// What [`apply`] wrote.
#[derive(Debug, Clone)]
pub struct Applied {
    pub workspace_id: String,
    pub workspace_dir: PathBuf,
    /// `<data>/workspaces/<id>/index.cartograph`.
    pub index: PathBuf,
}

/// Writes `cartograph.yaml` through a fresh temp file renamed over the name. The folder can be a
/// clone, so a link (or anything but a regular file) at that name is refused, never followed:
/// nothing outside the folder is written, and a crash never leaves half a manifest.
fn write_manifest(path: &Path, yaml: &str) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(std::io::Error::other(
            "it is a symlink or not a regular file; Cartograph writes only a regular file at that name",
        ));
    }
    let tmp = path.with_file_name(format!(".cartograph.yaml.{}.tmp", ids::new_workspace_id()));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(yaml.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Writes `cartograph.yaml`, the workspace folder and the registry entry. Running it again keeps the workspace id, so the index stays where it was.
pub fn apply(plan: &Plan, data: &Path) -> Result<Applied, DiscoveryError> {
    let werr = |p: &Path| {
        let p = p.display().to_string();
        move |e: std::io::Error| DiscoveryError::Write {
            path: p.clone(),
            why: e.to_string(),
        }
    };
    paths::create_private_dir(data).map_err(werr(data))?;
    paths::exclude_from_backup(data);
    // The manifest is meant to be committed, so it gets the user's normal permissions. One that
    // already says the same is left alone, comments and formatting included, so a teammate's init
    // never shows up as a diff.
    let same = manifest::read(&plan.manifest_path).is_ok_and(|p| p.manifest == plan.manifest);
    if !same {
        // Validated as a reader will see it, before anything is written: an invalid merge leaves
        // the committed file as it was.
        let yaml = manifest::to_yaml(&plan.manifest);
        manifest::parse(&yaml).map_err(|invalid| DiscoveryError::Manifest {
            path: plan.manifest_path.display().to_string(),
            invalid,
        })?;
        write_manifest(&plan.manifest_path, &yaml).map_err(werr(&plan.manifest_path))?;
    }

    let fresh = ids::new_workspace_id();
    let ws_entry = |id: String| WorkspaceEntry {
        id,
        name: plan.manifest.workspace.name.clone(),
        manifest: plan.manifest_path.clone(),
    };
    let id = Registry::update(data, |reg| {
        let id = reg
            .id_for_manifest(&plan.manifest_path)
            .map(str::to_owned)
            .unwrap_or(fresh);
        let repos = plan
            .repos
            .iter()
            .map(|r| RepoEntry {
                path: r.root.clone(),
                workspace: id.clone(),
                remote: r.remote.clone(),
                root_commit: r.root_commit.clone(),
            })
            .collect();
        reg.register(ws_entry(id.clone()), repos);
        id
    })?;
    let workspace_dir = paths::workspace_dir(data, &id);
    paths::create_private_dir(&workspace_dir).map_err(werr(&workspace_dir))?;
    Ok(Applied {
        index: workspace_dir.join("index.cartograph"),
        workspace_id: id,
        workspace_dir,
    })
}

/// The workspace that `dir` belongs to, through the registry: any subfolder of any of its repos,
/// or the folder that holds its manifest. `None` when no init covered it.
pub fn locate(data: &Path, dir: &Path) -> Result<Option<Applied>, DiscoveryError> {
    let (reg, _) = Registry::load(data)?;
    // The registry holds canonical paths: `/var` vs `/private/var`, `.`, relative folders.
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    // The folder init ran in holds cartograph.yaml and is often not inside any of the repos.
    let found = reg.workspace_for(&dir).or_else(|| {
        reg.workspaces
            .iter()
            .find(|w| w.manifest.parent() == Some(dir.as_path()))
    });
    Ok(found.map(|w| {
        let workspace_dir = paths::workspace_dir(data, &w.id);
        Applied {
            index: workspace_dir.join("index.cartograph"),
            workspace_id: w.id.clone(),
            workspace_dir,
        }
    }))
}
