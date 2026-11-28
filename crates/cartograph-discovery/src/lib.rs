//! Workspace file, repo checks, manifest, registry and default branch (C1-C5; IDX-08 to IDX-11).
//!
//! - [`workspace`]: find and read `.code-workspace` files (JSONC).
//! - [`resolve`]: folders to repos and scopes, with the reason for each skipped folder.
//! - [`branch`]: the default-branch ladder, with no network.
//! - [`manifest`]: `cartograph.yaml`, its schema and its config hash.
//! - [`registry`]: `~/.cartograph/registry.toml`, the allow-list of repos.
//! - [`setup`]: what `cartograph init` does before it indexes.
//! - [`config`]: `~/.cartograph/config.toml`, this machine's settings.

pub mod branch;
pub mod config;
pub mod manifest;
pub mod overrides;
pub mod registry;
pub mod resolve;
pub mod rules;
pub mod setup;
pub mod workspace;

pub use branch::{Source, default_branch};
pub use registry::Registry;
pub use resolve::{Resolution, ResolvedRepo, Scope, SkipReason, Skipped, resolve};
pub use workspace::{find_workspace_files, parse_workspace};

use cartograph_core::{ExitCode, Explain};

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("can't read the workspace file {path}")]
    WorkspaceFile { path: String, why: String },
    #[error("no workspace found in {dir}")]
    NoWorkspace { dir: String },
    #[error("{dir} holds {} workspace files", files.len())]
    ManyWorkspaces { dir: String, files: Vec<String> },
    #[error("can't find the default branch of {repo}")]
    Branch { repo: String, why: String },
    #[error("{path} isn't a valid manifest: {invalid}")]
    Manifest {
        path: String,
        invalid: manifest::Invalid,
    },
    #[error("can't update the registry at {path}")]
    Registry { path: String, why: String },
    #[error("can't write {path}")]
    Write { path: String, why: String },
    #[error("can't read the settings in {path}")]
    Config { path: String, why: String },
}

impl Explain for DiscoveryError {
    fn why(&self) -> String {
        match self {
            Self::WorkspaceFile { why, .. }
            | Self::Branch { why, .. }
            | Self::Registry { why, .. }
            | Self::Write { why, .. }
            | Self::Config { why, .. } => why.clone(),
            Self::NoWorkspace { .. } => {
                "there is no .code-workspace file here or two levels below, and no git repo within three levels".into()
            }
            Self::ManyWorkspaces { files, .. } => format!("found {}", files.join(", ")),
            Self::Manifest { invalid, .. } => invalid.message.clone(),
        }
    }
    fn action(&self) -> String {
        match self {
            Self::WorkspaceFile { .. } => "fix the file so VS Code opens it, or name another with --workspace".into(),
            Self::NoWorkspace { .. } => "run it in a folder with a .code-workspace file or a git repo, or pass --workspace <file>".into(),
            Self::ManyWorkspaces { files, .. } => format!("pick one: --workspace {}", files[0]),
            Self::Branch { .. } => "set default_branch for this repo in cartograph.yaml".into(),
            Self::Manifest { invalid, .. } => invalid.fix.clone(),
            Self::Registry { .. } | Self::Write { .. } => {
                "check the folder's permissions and the free disk space".into()
            }
            Self::Config { path, .. } => format!("fix {path}, or delete it to use the defaults"),
        }
    }
    fn exit_code(&self) -> ExitCode {
        match self {
            Self::NoWorkspace { .. } | Self::ManyWorkspaces { .. } => ExitCode::Usage,
            _ => ExitCode::Error,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// git in `dir` with no user or system config, so the machine's settings can't change a test.
    pub fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repo at `parent/name` on branch `branch` with one commit.
    pub fn repo_with_commit(parent: &Path, name: &str, branch: &str) -> PathBuf {
        let d = parent.join(name);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", branch]);
        std::fs::write(d.join("README.md"), "x\n").unwrap();
        git(&d, &["add", "."]);
        git(&d, &["commit", "-q", "-m", "first"]);
        d
    }
}
