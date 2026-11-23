//! ReplayFs: the tree at one replay step as a read-only file system for oxc_resolver (H1).
//!
//! It is backed by the replay's tree and the facts, never the disk, so a tsconfig `extends` of
//! `../../../../etc/x` or a `paths` entry outside the repo resolves inside the virtual tree or not at
//! all. Symlinks are absent from the tree; the only links are the `node_modules/<name>` links to
//! workspace packages, made as `pnpm install` would make them.
//!
//! While recording, every path the resolver asks about is logged. Those paths are the keys a
//! resolution depends on: a path whose existence or served content changes is the only thing that
//! can change the answer. Manifests are served as their resolution fields only, so the content
//! comparison is on those fields by construction (the phase 0 amendment).

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use oxc_resolver::{FileMetadata, FileSystem, ResolveError};

/// Where the tree is mounted. Paths outside it never exist.
pub const ROOT: &str = "/repo";

#[derive(Default)]
pub struct FsState {
    /// Path -> what `read` serves: a manifest's resolution JSON, or nothing for a source file.
    pub files: HashMap<String, Option<Arc<str>>>,
    /// Folder -> number of files and links below it. The root ("") always exists.
    pub dirs: HashMap<String, u32>,
    /// `node_modules/<name>` -> the package's folder.
    pub links: HashMap<String, String>,
    /// The paths asked about since recording started.
    pub log: Option<Vec<String>>,
}

impl FsState {
    pub(crate) fn note(&mut self, p: &str) {
        if let Some(l) = &mut self.log {
            l.push(p.to_owned());
        }
    }

    /// Follows a `node_modules/<name>` link at the start of the path, logging the link path.
    fn real(&mut self, rel: &str) -> String {
        if !rel.starts_with("node_modules/") {
            return rel.to_owned();
        }
        let parts: Vec<&str> = rel.split('/').collect();
        let n = if parts[1].starts_with('@') { 3 } else { 2 };
        if parts.len() < n {
            return rel.to_owned();
        }
        let link = parts[..n].join("/");
        self.note(&link);
        match self.links.get(&link) {
            Some(t) => join(t, &parts[n..].join("/")),
            None => rel.to_owned(),
        }
    }

    pub(crate) fn is_dir(&self, rel: &str) -> bool {
        rel.is_empty() || self.dirs.contains_key(rel)
    }

    /// Adds (+1) or removes (-1) one entry at `rel`; returns the folders whose existence changed.
    pub fn count_dirs(&mut self, rel: &str, d: i32, changed: &mut Vec<String>) {
        let mut p = rel;
        while let Some((parent, _)) = p.rsplit_once('/') {
            let n = self.dirs.entry(parent.to_owned()).or_default();
            *n = n.checked_add_signed(d).expect("dir counts balance");
            if *n == 0 {
                self.dirs.remove(parent);
                changed.push(parent.to_owned());
            } else if *n == 1 && d > 0 {
                changed.push(parent.to_owned());
            }
            p = parent;
        }
    }
}

fn join(dir: &str, rest: &str) -> String {
    let rest = rest.trim_start_matches('/');
    match (dir.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_owned(),
        (_, true) => dir.to_owned(),
        _ => format!("{dir}/{rest}"),
    }
}

/// `/repo/a/./b/../c` -> `a/c`; `None` outside the root.
pub fn to_rel(path: &Path) -> Option<String> {
    let rest = path.strip_prefix(ROOT).ok()?;
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

pub fn to_abs(rel: &str) -> PathBuf {
    if rel.is_empty() {
        PathBuf::from(ROOT)
    } else {
        Path::new(ROOT).join(rel)
    }
}

/// The resolver's view of an [`FsState`] shared with the replay.
pub struct ReplayFs(pub Arc<Mutex<FsState>>);

impl ReplayFs {
    fn state(&self) -> MutexGuard<'_, FsState> {
        self.0.lock().expect("not poisoned")
    }

    fn lookup(&self, path: &Path) -> Option<(String, Option<FileMetadata>)> {
        if path == Path::new("/") {
            return Some((String::new(), Some(FileMetadata::new(false, true, false))));
        }
        let rel = to_rel(path)?;
        let mut s = self.state();
        s.note(&rel);
        let real = s.real(&rel);
        if real != rel {
            s.note(&real);
        }
        let meta = if s.files.contains_key(&real) {
            Some(FileMetadata::new(true, false, false))
        } else if s.is_dir(&real) {
            Some(FileMetadata::new(false, true, false))
        } else {
            None
        };
        Some((real, meta))
    }
}

impl FileSystem for ReplayFs {
    fn new() -> Self {
        unreachable!("built with ReplayFs(state) and passed to new_with_file_system")
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let (real, _) = self.lookup(path).ok_or(io::ErrorKind::NotFound)?;
        match self.state().files.get(&real) {
            Some(Some(json)) => Ok(json.as_bytes().to_vec()),
            Some(None) => Ok(Vec::new()),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.lookup(path)
            .and_then(|(_, m)| m)
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if let Some(rel) = to_rel(path) {
            let mut s = self.state();
            s.note(&rel);
            if s.links.contains_key(&rel) {
                return Ok(FileMetadata::new(false, false, true));
            }
        }
        self.metadata(path)
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        let rel = to_rel(path).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut s = self.state();
        s.note(&rel);
        s.links
            .get(&rel)
            .map(|t| to_abs(t))
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput).into())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        match self.lookup(path) {
            Some((real, Some(_))) => Ok(to_abs(&real)),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
}
