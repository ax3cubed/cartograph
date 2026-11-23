//! `layout`: a folder of repositories in every shape discovery has to handle.
//!
//! - `monorepo/`: a pnpm monorepo (`packages/web` imports `@layout/core`), with two worktrees,
//!   `monorepo-feature/` and `monorepo-fix/`;
//! - `go-services/`: a `go.work` over two modules;
//! - `with-submodule/`: `go-services` as a submodule at `vendor/go-services`;
//! - `shallow/`: a depth-1 clone of `monorepo`; `bare.git/`: a bare clone of it;
//! - `notes/`: a folder that is not a repository;
//! - `layout.code-workspace`: comments, trailing commas, `~`, a `vscode-remote` uri and a duplicate.
//!
//! No network: the clones read the local `monorepo` over `file://`, and their remotes are removed.

use std::path::Path;
use std::process::Command;

use cartograph_git::fixture::Fixture;

pub(crate) const SOURCE: &str = include_str!("layout.rs");

pub const WORKTREES: [&str; 2] = ["monorepo-feature", "monorepo-fix"];

pub const CODE_WORKSPACE: &str = r#"// A workspace file with every quirk discovery must accept.
{
  "folders": [
    { "path": "monorepo" },
    { "path": "./monorepo/" }, // the same folder again
    { "name": "Go services", "path": "go-services" },
    { "path": "with-submodule" },
    { "path": "shallow" },
    { "path": "bare.git" },
    { "path": "monorepo-feature" },
    { "path": "~/code/not-on-this-machine" },
    { "uri": "vscode-remote://ssh-remote+box/srv/legacy-reports" },
    /* not a repository */
    { "path": "notes" },
  ],
  "settings": { "files.exclude": { "**/dist": true, }, },
}
"#;

/// The git command in `dir`, with no user or system config (as [`Fixture::git`]).
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "protocol.file.allow=always",
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

fn file_url(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

pub(crate) fn generate(root: &Path) {
    let mut mono = Fixture::init(&root.join("monorepo"));
    mono.write(
        "package.json",
        "{\n  \"name\": \"layout\",\n  \"private\": true\n}\n",
    );
    mono.write("pnpm-workspace.yaml", "packages:\n  - packages/*\n");
    mono.write(
        "packages/core/package.json",
        "{\n  \"name\": \"@layout/core\",\n  \"exports\": \"./src/index.ts\"\n}\n",
    );
    mono.write("packages/core/src/index.ts", "export const core = 1;\n");
    mono.commit("core package");
    mono.write(
        "packages/web/package.json",
        "{\n  \"name\": \"@layout/web\",\n  \"dependencies\": { \"@layout/core\": \"workspace:*\" }\n}\n",
    );
    mono.write(
        "packages/web/src/index.ts",
        "import { core } from \"@layout/core\";\nexport const web = core;\n",
    );
    mono.commit("web package");
    for (i, wt) in WORKTREES.iter().enumerate() {
        let branch = wt.trim_start_matches("monorepo-");
        mono.git(&["branch", branch, if i == 0 { "HEAD" } else { "HEAD~1" }]);
        mono.git(&["worktree", "add", "-q", &format!("../{wt}"), branch]);
    }

    let mut go = Fixture::init(&root.join("go-services"));
    go.write("go.work", "go 1.22\n\nuse (\n\t./auth\n\t./billing\n)\n");
    for m in ["auth", "billing"] {
        go.write(
            &format!("{m}/go.mod"),
            &format!("module example.com/{m}\n\ngo 1.22\n"),
        );
        go.write(&format!("{m}/main.go"), "package main\n\nfunc main() {}\n");
    }
    go.commit("two go modules");

    let mut sub = Fixture::init(&root.join("with-submodule"));
    sub.write("app/index.ts", "export const app = 1;\n");
    sub.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        "../go-services",
        "vendor/go-services",
    ]);
    sub.commit("app with a submodule");

    git(
        root,
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &file_url(&root.join("monorepo")),
            "shallow",
        ],
    );
    git(&root.join("shallow"), &["remote", "remove", "origin"]);
    git(root, &["clone", "-q", "--bare", "monorepo", "bare.git"]);
    git(&root.join("bare.git"), &["remote", "remove", "origin"]);

    std::fs::create_dir_all(root.join("notes")).expect("mkdir");
    std::fs::write(root.join("notes/todo.md"), "Not a repository.\n").expect("write");
    std::fs::write(root.join("layout.code-workspace"), CODE_WORKSPACE).expect("write");
}

/// Worktrees record absolute paths; point them at the folder's final place.
pub(crate) fn after_move(root: &Path) {
    let mut args = vec!["worktree".to_owned(), "repair".to_owned()];
    args.extend(WORKTREES.iter().map(|w| root.join(w).display().to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(&root.join("monorepo"), &args);
}
