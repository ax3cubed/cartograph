//! SRV-14, this phase's routes: preferences, the folder picker, discovery, adding a workspace, and
//! the workspace's `cartograph.yaml`. Corrections land with XS-13 and indexing settings later.
//!
//! The guard has already checked Host, Origin, the token and, for a write, `Content-Type:
//! application/json`. Every body is parsed against a closed schema (unknown fields are refused).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use cartograph_api::events::ServerEvent;
use cartograph_api::types::{
    Actor, ApiError, ConfigFile, ConfigWrite, CreateWorkspace, CreatedWorkspace, DiscoverRequest,
    DiscoveredFolder, Discovery, ErrorCode, FileDiff, FolderResult, FsEntry, IndexState, JobKind,
    Prefs, WorkspaceSummary,
};
use cartograph_core::{Explain, paths};
use cartograph_discovery::{DiscoveryError, manifest, setup};
use serde::Deserialize;

use crate::live;
use crate::routes::{App, body, error, ok};

fn bad(code: ErrorCode, what: &str, why: &str, action: &str) -> Response {
    error(&ApiError::new(code, what, why, action))
}

/// Runs `f` on the blocking pool; a panic becomes a plain 500.
async fn blocking<F>(f: F) -> Response
where
    F: FnOnce() -> Response + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "a write panicked");
        bad(
            ErrorCode::Internal,
            "something went wrong",
            "the server hit an internal error; the log has the details",
            "retry, then run cartograph report",
        )
    })
}

// ---------------------------------------------------------------- preferences (R1)

const PREFS: &str = "prefs.json";

/// `GET /api/v1/prefs`: the saved preferences, or the defaults.
pub async fn get_prefs(State(app): State<Arc<App>>) -> Response {
    let saved = std::fs::read(app.data_dir.join(PREFS))
        .ok()
        .and_then(|b| serde_json::from_slice::<Prefs>(&b).ok());
    ok(0, saved.unwrap_or_default())
}

/// `PUT /api/v1/prefs`: replaces them. Bounded so the file stays small.
pub async fn put_prefs(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    let p: Prefs = match body(&bytes) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // Characters, as the message says: a shortcut written with ⌘ or ⌥ is three bytes a key.
    let short =
        |s: &String| !s.is_empty() && s.chars().count() <= 64 && !s.chars().any(char::is_control);
    if ![90, 100, 115, 130].contains(&p.appearance.size)
        || p.keymap.len() > 200
        || !p.keymap.iter().all(|(k, v)| short(k) && short(v))
        || p.pins.len() > 200
        || p.pins.iter().any(|n| n.0.len() > 1024)
    {
        return bad(
            ErrorCode::Unprocessable,
            "these preferences can't be saved",
            "size is 90, 100, 115 or 130; at most 200 shortcuts of 1 to 64 characters and 200 pins",
            "reset the setting that changed and try again",
        );
    }
    let bytes = serde_json::to_vec_pretty(&p).expect("plain data");
    if let Err(e) = paths::create_private_dir(&app.data_dir)
        .and_then(|()| paths::write_private(&app.data_dir.join(PREFS), &bytes))
    {
        tracing::error!(error = %e, "can't write prefs.json");
        return bad(
            ErrorCode::Internal,
            "can't save preferences",
            "the data folder isn't writable",
            "check the data folder's permissions and free space",
        );
    }
    ok(0, p)
}

// ---------------------------------------------------------------- folders (R25)

#[derive(Deserialize)]
pub struct FsQuery {
    path: Option<String>,
}

/// The longest listing `fs/list` returns.
const MAX_ENTRIES: usize = 2000;

/// A typed path as an absolute, lexically clean path inside `home`: `~` expanded, a trailing
/// slash accepted, `..`, NUL and relative paths refused, and the canonical form must be the path
/// itself, so no symlink is followed anywhere along it.
fn allowed(home: Option<&Path>, typed: &str) -> Result<PathBuf, Response> {
    let refuse = |why: &str| {
        Err(bad(
            ErrorCode::BadRequest,
            "that folder can't be browsed",
            why,
            "type a folder inside your home folder, such as ~/code",
        ))
    };
    let Some(home) = home.and_then(|h| std::fs::canonicalize(h).ok()) else {
        return refuse("the server has no home folder to browse");
    };
    let typed = typed.trim();
    if typed.contains('\0') {
        return refuse("the path holds a NUL character");
    }
    let p = match typed {
        "" | "~" => home.clone(),
        t if t.starts_with("~/") || t.starts_with("~\\") => home.join(&t[2..]),
        t => PathBuf::from(t),
    };
    if !p.is_absolute() {
        return refuse("the path isn't absolute");
    }
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return refuse("the path holds . or ..");
    }
    // `components` drops a trailing slash and doubled separators.
    let p: PathBuf = p.components().collect();
    // Decided (phase 2 review): the home folder is the security boundary, not a convenience. A page
    // that steals the token can list folder names, so it sees only what the user owns; a repo
    // elsewhere is added with `cartograph init <dir>`. Mounted volumes would need a volumes list
    // the user writes in config.toml, not a wider picker.
    if !p.starts_with(&home) {
        return refuse("only folders inside your home folder can be browsed");
    }
    let Ok(canon) = std::fs::canonicalize(&p) else {
        return Err(bad(
            ErrorCode::NotFound,
            "no such folder",
            "nothing is at that path, or it can't be read",
            "check the path; ~ is your home folder",
        ));
    };
    if canon != p {
        return refuse("the path goes through a symbolic link");
    }
    if !canon.is_dir() {
        return refuse("that path isn't a folder");
    }
    Ok(canon)
}

fn visible_dirs(dir: &Path) -> impl Iterator<Item = String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        // `file_type` doesn't follow symlinks, so a link to a folder isn't one.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
}

/// `GET /api/v1/fs/list?path=`: the folders in `path`, by name. Never a file's name or contents,
/// and nothing inside a child: on macOS a look inside Desktop, Documents or Downloads raises a
/// privacy prompt the user didn't ask for. The flags stay unknown; the picker lists a folder when
/// it is opened.
pub async fn fs_list(State(app): State<Arc<App>>, q: Query<FsQuery>) -> Response {
    let typed = q.0.path.unwrap_or_default();
    blocking(move || {
        let dir = match allowed(app.home.as_deref(), &typed) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut out: Vec<FsEntry> = visible_dirs(&dir)
            .take(MAX_ENTRIES)
            .map(|name| FsEntry {
                is_repo: None,
                has_workspace_file: None,
                has_children: None,
                name,
            })
            .collect();
        out.sort_by_key(|e| e.name.to_lowercase());
        ok(0, out)
    })
    .await
}

// ---------------------------------------------------------------- discovery and workspaces

/// The folder and workspace file of a discover or create request, checked like `fs/list`.
fn target(
    home: Option<&Path>,
    path: &str,
    file: Option<&str>,
) -> Result<(PathBuf, Option<PathBuf>), Response> {
    let dir = allowed(home, path)?;
    let file = match file.map(str::trim).filter(|f| !f.is_empty()) {
        None => None,
        Some(f) => {
            let f = if f.starts_with('~') || Path::new(f).is_absolute() {
                PathBuf::from(f)
            } else {
                dir.join(f)
            };
            let f = f.to_string_lossy().into_owned();
            let parent = Path::new(&f).parent().unwrap_or(Path::new("/"));
            let parent = allowed(home, &parent.to_string_lossy())?;
            let name = Path::new(&f).file_name().unwrap_or_default();
            if !name.to_string_lossy().ends_with(".code-workspace") {
                return Err(bad(
                    ErrorCode::BadRequest,
                    "that isn't a workspace file",
                    "a workspace file's name ends in .code-workspace",
                    "type the .code-workspace file's name, or leave it empty",
                ));
            }
            Some(parent.join(name))
        }
    };
    Ok((dir, file))
}

/// A discovery error as E1. Several workspace files in one folder: the user types one.
fn discovery_error(e: &DiscoveryError) -> Response {
    if let DiscoveryError::ManyWorkspaces { files, .. } = e {
        let names: Vec<String> = files
            .iter()
            .filter_map(|f| Path::new(f).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        return bad(
            ErrorCode::Unprocessable,
            &format!("this folder holds {} workspace files", files.len()),
            &format!("Cartograph can't tell which to use: {}", names.join(", ")),
            "type one of them in the workspace file field",
        );
    }
    bad(ErrorCode::Unprocessable, &e.what(), &e.why(), &e.action())
}

fn rows(plan: &setup::Plan) -> Vec<DiscoveredFolder> {
    let mut out = Vec::new();
    for (r, m) in plan.resolution.repos.iter().zip(&plan.manifest.repos) {
        if r.scopes.is_empty() {
            out.push(DiscoveredFolder {
                folder: m.path.clone(),
                name: m.display.clone().unwrap_or_else(|| m.name.clone()),
                result: FolderResult::Repo,
                repo: Some(m.name.clone()),
                reason: None,
            });
        }
        for s in &r.scopes {
            out.push(DiscoveredFolder {
                folder: format!("{}/{}", m.path.trim_end_matches('/'), s.prefix),
                name: s.name.clone().unwrap_or_else(|| s.prefix.clone()),
                result: FolderResult::Scope,
                repo: Some(m.name.clone()),
                reason: None,
            });
        }
    }
    out.extend(plan.resolution.skipped.iter().map(|s| DiscoveredFolder {
        folder: s.folder.clone(),
        name: s.folder.clone(),
        result: FolderResult::Skipped,
        repo: None,
        reason: Some(s.reason.as_str().to_owned()),
    }));
    out
}

/// `POST /api/v1/discover`: the C1 and C2 resolution table. Reads git metadata only; writes
/// nothing.
pub async fn discover(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    let req: DiscoverRequest = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    blocking(move || {
        let (dir, file) = match target(
            app.home.as_deref(),
            &req.path,
            req.workspace_file.as_deref(),
        ) {
            Ok(t) => t,
            Err(e) => return e,
        };
        match setup::plan(&dir, file.as_deref()) {
            Ok(plan) => ok(
                0,
                Discovery {
                    folders: rows(&plan),
                    // ponytail: services come with IDX-20, the estimate with a measured rate.
                    services: vec![],
                    estimate_s: None,
                },
            ),
            Err(e) => discovery_error(&e),
        }
    })
    .await
}

/// `POST /api/v1/workspaces`: writes `cartograph.yaml` and the registry entry (as `cartograph
/// init` does), then starts the first index. Its progress arrives on `/api/v1/events`; cancelling
/// that job removes the workspace again.
// ponytail: `manifest: false` still writes cartograph.yaml, because the registry keys a workspace
// by its manifest's path (IDX-11); it is accepted so the wizard's request stays valid.
pub async fn create_workspace(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    let req: CreateWorkspace = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if let Some(n) = &req.name
        && (n.trim().is_empty() || n.chars().count() > 100 || n.chars().any(char::is_control))
    {
        return bad(
            ErrorCode::BadRequest,
            "that name can't be used",
            "a workspace name is 1 to 100 characters, without control characters",
            "type a shorter name, or leave it empty to use the folder's",
        );
    }
    let a = app.clone();
    blocking(move || {
        let (dir, file) = match target(a.home.as_deref(), &req.path, req.workspace_file.as_deref())
        {
            Ok(t) => t,
            Err(e) => return e,
        };
        let mut plan = match setup::plan(&dir, file.as_deref()) {
            Ok(p) => p,
            Err(e) => return discovery_error(&e),
        };
        if plan.repos.is_empty() {
            return bad(
                ErrorCode::Unprocessable,
                "no repository to index here",
                "every folder of this workspace was skipped",
                "check the table from discover; each row says why",
            );
        }
        if let Some(n) = &req.name {
            plan.manifest.workspace.name = n.trim().to_owned();
        }
        // Anything already at the name (a committed manifest, even a link) is never removed.
        let had_manifest = std::fs::symlink_metadata(&plan.manifest_path).is_ok();
        let applied = match setup::apply(&plan, &a.data_dir) {
            Ok(x) => x,
            Err(e) => return discovery_error(&e),
        };
        let ws = applied.workspace_id;
        let first = !applied.index.exists();
        // A cancelled first index removes what this request added, the manifest it wrote included.
        let created = (!had_manifest).then(|| plan.manifest_path.clone());
        let job = live::start_with(&a, &ws, JobKind::Sync, Actor::App, first, created);
        a.live.emit(ServerEvent::WorkspacesChanged {});
        ok(
            0,
            CreatedWorkspace {
                workspace: WorkspaceSummary {
                    id: ws.clone(),
                    name: plan.manifest.workspace.name.clone(),
                    path: plan
                        .manifest_path
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    demo: false,
                    repos: plan.repos.len() as u32,
                    services: 0,
                    modules: 0,
                    files: 0,
                    commits: 0,
                    state: IndexState::Syncing,
                    built_at: None,
                },
                job_id: job.id,
            },
        )
    })
    .await
}

// ---------------------------------------------------------------- cartograph.yaml

/// The registered manifest of `ws`, refusing a symlink at its name.
fn manifest_of(app: &App, ws: &str) -> Result<PathBuf, Response> {
    let p = crate::sync::manifest_path(&app.data_dir, ws).ok_or_else(|| {
        bad(
            ErrorCode::NotFound,
            "no such workspace",
            "the registry has no workspace with this id",
            "list workspaces with GET /api/v1/workspaces",
        )
    })?;
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(bad(
            ErrorCode::Unprocessable,
            "cartograph.yaml is a symbolic link",
            "Cartograph writes only a regular file at that name",
            "replace the link with the file itself",
        ));
    }
    Ok(p)
}

fn etag(bytes: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    format!("\"{:016x}\"", h.finish())
}

/// A line diff in unified form, without hunk headers. Over 4M line pairs it shows the whole file
/// replaced, which is still correct.
fn diff(old: &str, new: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let mut out = String::from("--- a/cartograph.yaml\n+++ b/cartograph.yaml\n");
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        a.iter().for_each(|l| out.push_str(&format!("-{l}\n")));
        b.iter().for_each(|l| out.push_str(&format!("+{l}\n")));
        return out;
    }
    // Longest common subsequence, then walk it.
    let mut t = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push_str(&format!(" {}\n", a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || t[i + 1][j] >= t[i][j + 1]) {
            out.push_str(&format!("-{}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+{}\n", b[j]));
            j += 1;
        }
    }
    out
}

/// Parses a proposed manifest with the same checks `cartograph init` and sync use.
fn check_yaml(yaml: &str) -> Result<(), Response> {
    if yaml.len() > manifest::MAX_MANIFEST {
        return Err(bad(
            ErrorCode::PayloadTooLarge,
            "cartograph.yaml is too large",
            "the manifest is at most 1 MB",
            "move long lists out of the manifest",
        ));
    }
    let parsed = manifest::parse(yaml).map_err(|e| {
        bad(
            ErrorCode::Unprocessable,
            "cartograph.yaml isn't valid",
            &e.message,
            &e.fix,
        )
    })?;
    // Sync refuses a repo name it can't use as a path prefix; the manifest's own checks are looser,
    // so a saved name like `My API` would fail every sync after it instead of this request.
    if let Some(r) = parsed
        .manifest
        .repos
        .iter()
        .find(|r| !cartograph_index::timeline::valid_repo_name(&r.name))
    {
        return Err(bad(
            ErrorCode::Unprocessable,
            "cartograph.yaml isn't valid",
            &format!("`{}` can't be used as a repository name", r.name),
            "use ASCII letters, digits, '.', '_' and '-', at most 64, not starting with '.'",
        ));
    }
    Ok(())
}

/// `GET /api/v1/w/{ws}/config`.
pub async fn get_config(State(app): State<Arc<App>>, UrlPath(ws): UrlPath<String>) -> Response {
    blocking(move || {
        let p = match manifest_of(&app, &ws) {
            Ok(p) => p,
            Err(e) => return e,
        };
        match std::fs::read(&p) {
            Ok(b) => ok(
                0,
                ConfigFile {
                    etag: etag(&b),
                    yaml: String::from_utf8_lossy(&b).into_owned(),
                },
            ),
            Err(_) => bad(
                ErrorCode::NotFound,
                "cartograph.yaml is missing",
                "the workspace's manifest isn't where the registry says",
                "run cartograph init in the workspace folder",
            ),
        }
    })
    .await
}

/// `POST /api/v1/w/{ws}/config/preview`: the diff a write would make. Changes nothing.
pub async fn preview_config(
    State(app): State<Arc<App>>,
    UrlPath(ws): UrlPath<String>,
    bytes: Bytes,
) -> Response {
    let req: ConfigWrite = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    blocking(move || {
        let p = match manifest_of(&app, &ws) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if let Err(e) = check_yaml(&req.yaml) {
            return e;
        }
        let old = std::fs::read_to_string(&p).unwrap_or_default();
        ok(
            0,
            FileDiff {
                file: manifest::FILE_NAME.into(),
                diff: diff(&old, &req.yaml),
                applied: false,
            },
        )
    })
    .await
}

/// `PUT /api/v1/w/{ws}/config` with `If-Match`: writes `cartograph.yaml` through a temporary file
/// and a rename, then syncs. 412 when the file changed since it was read.
pub async fn put_config(
    State(app): State<Arc<App>>,
    UrlPath(ws): UrlPath<String>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let req: ConfigWrite = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let if_match = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let a = app.clone();
    blocking(move || {
        let p = match manifest_of(&a, &ws) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if let Err(e) = check_yaml(&req.yaml) {
            return e;
        }
        // Optimistic concurrency: the If-Match check and the rename are one step, so of two saves
        // made from the same version one lands and the other gets 412, never a lost update. Each
        // save writes its own temp file.
        // ponytail: one lock for every workspace's saves; per-workspace locks if saves contend.
        static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let saving = SAVING.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::fs::read(&p).unwrap_or_default();
        if if_match.as_deref() != Some(etag(&old).as_str()) {
            return bad(
                ErrorCode::PreconditionFailed,
                "cartograph.yaml changed on disk",
                "it was edited after the app read it, or If-Match is missing",
                "reload the settings, then make the change again",
            );
        }
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = p.with_file_name(format!(".cartograph.yaml.{}.{n}.tmp", std::process::id()));
        let written = std::fs::write(&tmp, &req.yaml).and_then(|()| std::fs::rename(&tmp, &p));
        drop(saving);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            tracing::error!(error = %e, "can't write cartograph.yaml");
            return bad(
                ErrorCode::Internal,
                "can't write cartograph.yaml",
                "the workspace folder isn't writable",
                "check the folder's permissions",
            );
        }
        let tag = etag(req.yaml.as_bytes());
        a.live.emit(ServerEvent::ConfigChanged {
            ws: ws.clone(),
            etag: tag,
            file: manifest::FILE_NAME.into(),
        });
        // A repo the user added here is registered (inside home, a git repo); any other stays out
        // of sync (F8) and freshness reports it as not added.
        crate::sync::register_chosen(&a.data_dir, &ws, a.home.as_deref());
        // C7: the manifest decides what is indexed, so a change syncs.
        live::start(&a, &ws, JobKind::Sync, Actor::App, false);
        ok(
            0,
            FileDiff {
                file: manifest::FILE_NAME.into(),
                diff: diff(&String::from_utf8_lossy(&old), &req.yaml),
                applied: true,
            },
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_diff_keeps_common_lines() {
        let d = diff("a\nb\nc\n", "a\nx\nc\n");
        assert_eq!(
            d,
            "--- a/cartograph.yaml\n+++ b/cartograph.yaml\n a\n-b\n+x\n c\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_clean_paths_inside_home_are_browsable() {
        let d = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(d.path()).unwrap();
        std::fs::create_dir_all(home.join("code/a")).unwrap();
        std::os::unix::fs::symlink("/etc", home.join("link")).unwrap();
        let h = Some(home.as_path());
        assert_eq!(allowed(h, "~").ok(), Some(home.clone()));
        assert_eq!(allowed(h, "~/code/").ok(), Some(home.join("code")));
        let full = format!("{}/code/a/", home.display());
        assert_eq!(allowed(h, &full).ok(), Some(home.join("code/a")));
        for bad in [
            "~/code/../..",
            "~/link",
            "~/link/",
            "/etc",
            "/proc/self",
            "code",
            "~/code/\0",
            "//server/share",
            "~/missing",
        ] {
            assert!(allowed(h, bad).is_err(), "{bad}");
        }
        assert!(allowed(None, "/").is_err());
    }
}
