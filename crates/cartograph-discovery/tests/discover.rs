//! IDX-08 against real repositories: the spec's acme example, the layout fixture, folder order and
//! the 50-folder budget.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use cartograph_discovery::resolve::resolve_folders;
use cartograph_discovery::workspace::{WorkspaceFile, parse_workspace_str};
use cartograph_discovery::{Scope, SkipReason, Skipped, find_workspace_files, resolve};
use proptest::prelude::*;

fn git(dir: &Path, args: &[&str]) {
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

fn repo(dir: &Path, files: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x\n").unwrap();
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
}

fn skipped(folder: &str, reason: SkipReason) -> Skipped {
    Skipped {
        folder: folder.into(),
        reason,
    }
}

/// Spec §1: the acme.code-workspace example gives exactly its result table.
#[test]
fn the_spec_acme_example_yields_the_spec_table() {
    let t = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(t.path()).unwrap();
    let acme = root.join("code/acme");
    repo(&acme.join("billing-api"), &["src/index.ts"]);
    repo(&acme.join("web"), &["src/index.ts"]);
    repo(
        &acme.join("platform"),
        &[
            "packages/auth/index.ts",
            "packages/events/index.ts",
            "packages/other/index.ts",
        ],
    );
    let protos = root.join("src/shared-protos");
    repo(&protos, &["a.proto"]);
    std::fs::create_dir_all(acme.join("notes")).unwrap();
    let text = format!(
        r#"// Acme platform
{{
  "folders": [
    {{ "path": "billing-api" }},
    {{ "path": "web", "name": "Storefront" }},
    {{ "path": "platform/packages/auth" }},
    {{ "path": "platform/packages/events" }},
    {{ "path": "{}" }},
    {{ "uri": "vscode-remote://ssh-remote+box/srv/legacy" }},
    {{ "path": "notes" }},
  ],
  "settings": {{
    "files.exclude": {{ "**/dist": true }}
  }}
}}"#,
        protos.display()
    );
    let file = acme.join("acme.code-workspace");
    std::fs::write(&file, text).unwrap();

    assert_eq!(
        find_workspace_files(&root.join("code")),
        std::slice::from_ref(&file)
    );
    let r = resolve(&file).unwrap();
    let table: Vec<_> = r
        .repos
        .iter()
        .map(|x| {
            (
                x.root.clone(),
                x.name.as_str(),
                x.display.as_deref(),
                x.scopes.clone(),
            )
        })
        .collect();
    let scope = |p: &str| Scope {
        prefix: p.into(),
        name: None,
    };
    assert_eq!(
        table,
        [
            (acme.join("billing-api"), "billing-api", None, vec![]),
            (
                acme.join("platform"),
                "platform",
                None,
                vec![scope("packages/auth"), scope("packages/events")]
            ),
            (acme.join("web"), "web", Some("Storefront"), vec![]),
            (protos.clone(), "shared-protos", None, vec![]),
        ]
    );
    assert_eq!(
        r.skipped,
        [
            skipped(
                "vscode-remote://ssh-remote+box/srv/legacy",
                SkipReason::NotOnThisMachine
            ),
            skipped("notes", SkipReason::NotAGitRepository),
        ]
    );
    assert_eq!(r.files_exclude, ["**/dist"]);
    assert!(r.repos.iter().all(|x| !x.shallow));
}

#[test]
fn the_layout_fixture_resolves_every_shape() {
    let layout = cartograph_fixtures::layout();
    let file = layout.join("layout.code-workspace");
    assert_eq!(find_workspace_files(&layout), std::slice::from_ref(&file));
    let r = resolve(&file).unwrap();
    let layout = std::fs::canonicalize(&layout).unwrap();
    let repos: Vec<_> = r
        .repos
        .iter()
        .map(|x| {
            (
                x.root.strip_prefix(&layout).unwrap().to_owned(),
                x.display.clone(),
                x.shallow,
                x.scopes.len(),
            )
        })
        .collect();
    assert_eq!(
        repos,
        [
            (
                PathBuf::from("go-services"),
                Some("Go services".into()),
                false,
                0
            ),
            // monorepo-feature is a worktree of monorepo: one repo, rooted at the main worktree.
            (PathBuf::from("monorepo"), None, false, 0),
            (PathBuf::from("shallow"), None, true, 0),
            (PathBuf::from("with-submodule"), None, false, 0),
        ]
    );
    assert_eq!(
        r.skipped,
        [
            skipped("./monorepo/", SkipReason::Duplicate),
            skipped("bare.git", SkipReason::BareRepository),
            skipped("~/code/not-on-this-machine", SkipReason::Missing),
            skipped(
                "vscode-remote://ssh-remote+box/srv/legacy-reports",
                SkipReason::NotOnThisMachine
            ),
            skipped("notes", SkipReason::NotAGitRepository),
        ]
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("~/code/not-on-this-machine")),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings.iter().any(|w| w.contains("shallow")),
        "{:?}",
        r.warnings
    );
}

fn layout_ws(folders: &[(&str, Option<&str>)]) -> WorkspaceFile {
    let list: Vec<String> = folders
        .iter()
        .map(|(p, n)| match n {
            Some(n) => format!(r#"{{"path":"{p}","name":"{n}"}}"#),
            None => format!(r#"{{"path":"{p}"}}"#),
        })
        .collect();
    parse_workspace_str(&format!(r#"{{"folders":[{}]}}"#, list.join(","))).unwrap()
}

#[test]
fn a_submodule_is_its_own_repo_only_when_listed() {
    let layout = cartograph_fixtures::layout();
    let r = resolve_folders(&layout, &layout_ws(&[("with-submodule", None)]));
    assert_eq!(r.repos.len(), 1);
    let r = resolve_folders(
        &layout,
        &layout_ws(&[
            ("with-submodule", None),
            ("with-submodule/vendor/go-services", None),
        ]),
    );
    let names: Vec<_> = r.repos.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["with-submodule", "go-services"]);
    assert!(r.repos[1].scopes.is_empty());
}

#[test]
fn worktrees_alone_group_and_subfolders_become_named_scopes() {
    let layout = cartograph_fixtures::layout();
    let r = resolve_folders(
        &layout,
        &layout_ws(&[("monorepo-fix", None), ("monorepo-feature", None)]),
    );
    assert_eq!(r.repos.len(), 1);
    assert_eq!(r.repos[0].name, "monorepo-feature");
    let r = resolve_folders(
        &layout,
        &layout_ws(&[
            ("monorepo/packages/web", Some("Web")),
            ("monorepo-fix/packages/core", None),
        ]),
    );
    assert_eq!(r.repos.len(), 1);
    assert_eq!(r.repos[0].name, "monorepo");
    let scopes: Vec<_> = r.repos[0]
        .scopes
        .iter()
        .map(|s| (s.prefix.as_str(), s.name.as_deref()))
        .collect();
    assert_eq!(
        scopes,
        [("packages/core", None), ("packages/web", Some("Web"))]
    );
}

#[test]
fn backslashes_file_uris_and_remote_workspaces() {
    let layout = std::fs::canonicalize(cartograph_fixtures::layout()).unwrap();
    let uri = format!(
        "file://{}",
        layout
            .join("go-services")
            .display()
            .to_string()
            .replace(' ', "%20")
    );
    let ws = parse_workspace_str(&format!(
        r#"{{"folders":[{{"path":"monorepo\\packages\\web"}},{{"uri":"{uri}"}}]}}"#
    ))
    .unwrap();
    let r = resolve_folders(&layout, &ws);
    assert_eq!(r.repos.len(), 2, "{r:?}");
    assert_eq!(r.repos[1].scopes[0].prefix, "packages/web");
    let remote =
        parse_workspace_str(r#"{"folders":[{"path":"monorepo"}],"remoteAuthority":"wsl+Ubuntu"}"#)
            .unwrap();
    let r = resolve_folders(&layout, &remote);
    assert!(r.repos.is_empty());
    assert_eq!(
        r.skipped,
        [skipped("monorepo", SkipReason::RemoteWorkspace)]
    );
}

#[cfg(unix)]
#[test]
fn the_workspace_search_never_follows_symlinks() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("x.code-workspace"), "{}").unwrap();
    let dir = t.path().join("dir");
    std::fs::create_dir_all(dir.join("node_modules")).unwrap();
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    std::fs::write(dir.join("node_modules/n.code-workspace"), "{}").unwrap();
    std::fs::write(dir.join(".hidden/h.code-workspace"), "{}").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("x.code-workspace"),
        dir.join("l.code-workspace"),
    )
    .unwrap();
    assert!(find_workspace_files(&dir).is_empty());
}

#[test]
fn fifty_folders_resolve_within_500_ms() {
    let t = tempfile::tempdir().unwrap();
    let mut list = Vec::new();
    for i in 0..25 {
        let name = format!("r{i:02}");
        repo(&t.path().join(&name), &["a/x.ts", "b/y.ts"]);
        list.push(format!(r#"{{"path":"{name}/a"}},{{"path":"{name}/b"}}"#));
    }
    let ws = parse_workspace_str(&format!(r#"{{"folders":[{}]}}"#, list.join(","))).unwrap();
    assert_eq!(ws.folders.len(), 50);
    let t0 = Instant::now();
    let r = resolve_folders(t.path(), &ws);
    let ms = t0.elapsed().as_millis();
    assert_eq!(r.repos.len(), 25);
    assert!(r.repos.iter().all(|x| x.scopes.len() == 2));
    eprintln!("50 folders in {ms} ms");
    assert!(ms < 500, "50 folders took {ms} ms");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn folder_order_doesnt_change_the_result(
        order in Just((0..10usize).collect::<Vec<_>>()).prop_shuffle()
    ) {
        let layout = cartograph_fixtures::layout();
        let ws = cartograph_discovery::parse_workspace(&layout.join("layout.code-workspace")).unwrap();
        let base = resolve_folders(&layout, &ws);
        let shuffled = WorkspaceFile {
            folders: order.iter().map(|&i| ws.folders[i].clone()).collect(),
            ..ws.clone()
        };
        let got = resolve_folders(&layout, &shuffled);
        prop_assert_eq!(&got.repos, &base.repos);
        let reasons = |s: &[Skipped]| {
            let mut v: Vec<_> = s.iter().map(|x| x.reason).collect();
            v.sort();
            v
        };
        prop_assert_eq!(reasons(&got.skipped), reasons(&base.skipped));
    }
}
