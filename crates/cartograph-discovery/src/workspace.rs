//! Finding and reading `.code-workspace` files (C1).
//!
//! The file is JSONC: comments and trailing commas, as VS Code allows. Only `folders`,
//! `remoteAuthority` and `settings["files.exclude"]` are read; everything else is ignored.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::DiscoveryError;

/// The JSONC input is capped, so a hostile or mistaken file can't make discovery read gigabytes.
pub const MAX_WORKSPACE_FILE: u64 = 5 * 1024 * 1024;

/// Folders a workspace search never enters (C2).
const SKIP: [&str; 3] = ["node_modules", "target", ".venv"];

/// One entry of `folders[]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FolderEntry {
    pub path: Option<String>,
    pub uri: Option<String>,
    pub name: Option<String>,
}

/// What Cartograph reads from a `.code-workspace` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceFile {
    pub folders: Vec<FolderEntry>,
    /// Set when the whole workspace lives on another machine; the file is then skipped.
    pub remote_authority: Option<String>,
    /// The `files.exclude` globs whose value is `true`, sorted.
    pub files_exclude: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    folders: Vec<FolderEntry>,
    #[serde(rename = "remoteAuthority")]
    remote_authority: Option<String>,
    #[serde(default)]
    settings: Settings,
}

#[derive(Deserialize, Default)]
struct Settings {
    #[serde(rename = "files.exclude", default)]
    files_exclude: serde_json::Map<String, serde_json::Value>,
}

/// Parses the text of a `.code-workspace` file.
pub fn parse_workspace_str(text: &str) -> Result<WorkspaceFile, String> {
    // Windows tools (PowerShell 5's Out-File, Visual Studio) save UTF-8 with a BOM, which VS Code
    // reads as whitespace and serde_json refuses.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut s = text.to_owned();
    // Comments and trailing commas become spaces, so serde_json's line and column stay true.
    json_strip_comments::strip(&mut s).map_err(|e| e.to_string())?;
    let raw: Raw = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    let mut files_exclude: Vec<String> = raw
        .settings
        .files_exclude
        .into_iter()
        .filter(|(_, v)| v == &serde_json::Value::Bool(true))
        .map(|(k, _)| k)
        .collect();
    files_exclude.sort();
    Ok(WorkspaceFile {
        folders: raw.folders,
        remote_authority: raw.remote_authority.filter(|r| !r.is_empty()),
        files_exclude,
    })
}

/// Reads and parses a `.code-workspace` file, refusing one over [`MAX_WORKSPACE_FILE`].
pub fn parse_workspace(file: &Path) -> Result<WorkspaceFile, DiscoveryError> {
    let bad = |why: String| DiscoveryError::WorkspaceFile {
        path: file.display().to_string(),
        why,
    };
    let meta = std::fs::metadata(file).map_err(|e| bad(e.to_string()))?;
    // A named file can be a link to a device or a pipe, which reports size 0 and never ends.
    if !meta.is_file() {
        return Err(bad("it isn't a regular file".into()));
    }
    let len = meta.len();
    if len > MAX_WORKSPACE_FILE {
        return Err(bad(format!("it is {len} bytes; the limit is 5 MB")));
    }
    let mut text = String::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_WORKSPACE_FILE + 1).read_to_string(&mut text))
        .map_err(|e| bad(e.to_string()))?;
    if text.len() as u64 > MAX_WORKSPACE_FILE {
        return Err(bad("it grew past the 5 MB limit while it was read".into()));
    }
    parse_workspace_str(&text).map_err(bad)
}

/// The `*.code-workspace` files in `dir` and up to two levels below, sorted. Skips hidden folders,
/// `node_modules` and the like, and never follows a symlink.
pub fn find_workspace_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        // DirEntry::file_type doesn't follow symlinks, so a symlink is neither a dir nor a file here.
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name();
        let name = name.to_string_lossy();
        if ft.is_file() && name.ends_with(".code-workspace") {
            out.push(e.path());
        } else if ft.is_dir() && depth < 2 && !name.starts_with('.') && !SKIP.contains(&&*name) {
            walk(&e.path(), depth + 1, out);
        }
    }
}

/// With no workspace file (C2): the repos in `dir` and up to three levels below, as folder entries.
/// Stops at the first `.git` on each branch, skips hidden folders and `node_modules`, `target`,
/// `.venv`, and never follows a symlink. If `dir` is itself inside a repo, that is the one folder.
// ponytail: doesn't read .gitignore while walking; an ignored folder holding a repo is listed. The
// upgrade is the ignore crate's walker, as C2 names.
pub fn folders_without_workspace_file(dir: &Path) -> Vec<FolderEntry> {
    let entry = |p: &Path| FolderEntry {
        path: Some(p.display().to_string()),
        uri: None,
        name: None,
    };
    if crate::resolve::discover(dir).is_some() {
        return vec![entry(dir)];
    }
    let mut repos = Vec::new();
    find_repos(dir, 0, &mut repos);
    repos.sort();
    repos.iter().map(|p| entry(p)).collect()
}

fn find_repos(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join(".git").exists() {
        out.push(dir.to_owned());
        return;
    }
    if depth == 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if e.file_type().is_ok_and(|t| t.is_dir())
            && !name.starts_with('.')
            && !SKIP.contains(&&*name)
        {
            find_repos(&e.path(), depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_is_skipped_as_vs_code_does() {
        let w = parse_workspace_str("\u{feff}{ \"folders\": [ { \"path\": \"api\" } ] }").unwrap();
        assert_eq!(w.folders.len(), 1);
    }

    #[test]
    fn comments_trailing_commas_and_only_true_excludes() {
        let w = parse_workspace_str(
            r#"// head
            { "folders": [ { "path": "a" }, /* x */ { "uri": "file:///b", "name": "B" }, ],
              "settings": { "files.exclude": { "**/dist": true, "**/tmp": false, },
                            "search.exclude": { "**/x": true } },
              "extensions": { "recommendations": [] }, }"#,
        )
        .unwrap();
        assert_eq!(w.folders.len(), 2);
        assert_eq!(w.folders[1].name.as_deref(), Some("B"));
        assert_eq!(w.files_exclude, ["**/dist"]);
        assert_eq!(w.remote_authority, None);
    }

    #[test]
    fn remote_authority_is_read() {
        let w =
            parse_workspace_str(r#"{"folders":[],"remoteAuthority":"ssh-remote+box"}"#).unwrap();
        assert_eq!(w.remote_authority.as_deref(), Some("ssh-remote+box"));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for s in ["", "{", "[]", "{\"folders\": 3}", "/* open", "\u{0}"] {
            assert!(parse_workspace_str(s).is_err(), "{s:?}");
        }
    }

    proptest::proptest! {
        // ponytail: a property test stands in for the fuzz target until cargo-fuzz lands (HD-06).
        #[test]
        fn the_reader_never_panics(s in "\\PC{0,200}") {
            let _ = parse_workspace_str(&s);
        }
    }
}
