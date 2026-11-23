//! `cartograph status` (SRV-03): `--check` exits 3 on a repo one commit behind, `--line` is one
//! short line of counts and fixed words that exits 0 even with no workspace.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Registers and indexes `dir` as `cartograph init` does.
fn init(data: &Path, dir: &Path) {
    let plan = cartograph_discovery::setup::plan(dir, None).unwrap();
    let applied = cartograph_discovery::setup::apply(&plan, data).unwrap();
    let ws = cartograph_server::workspace(data, &applied.workspace_id).unwrap();
    let mut cache = FactCache::open(&data.join("cache/facts.db")).unwrap();
    cartograph_index::sync::sync(&ws, &Default::default(), &mut cache).unwrap();
}

#[test]
fn check_exits_3_when_one_commit_behind() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let mut f = Fixture::init(&root.join("private-name"));
    f.write("a.ts", "import './b';");
    f.write("b.ts", "");
    f.commit("one");
    init(&data, &f.dir);

    let o = cartograph(&data, &["status", "--check", f.dir.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("behind 0"));

    f.write("c.ts", "");
    f.commit("two");
    // Past the 1 s memo of the freshness check.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = cartograph(
        &data,
        &["status", "--check", "--json", f.dir.to_str().unwrap()],
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["state"], "behind");
    assert_eq!(v["repos"][0]["behind"], 1);
    assert!(String::from_utf8_lossy(&o.stderr).contains("cartograph sync"));

    let o = cartograph(&data, &["status", "--line", f.dir.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let line = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        line.trim_end(),
        "cartograph: 1 commits behind in 1 of 1 repos"
    );
    assert!(line.lines().count() == 1 && line.len() < 120 && !line.contains("private-name"));
}

/// `acme/acme.code-workspace` over the repos named, each with one commit.
fn workspace(root: &Path, repos: &[&str]) -> std::path::PathBuf {
    let ws = root.join("acme");
    for name in repos {
        if !ws.join(name).exists() {
            let mut f = Fixture::init(&ws.join(name));
            f.write("a.ts", "export const a = 1;");
            f.commit("one");
        }
    }
    let folders: Vec<String> = repos
        .iter()
        .map(|n| format!(r#"{{ "path": "{n}" }}"#))
        .collect();
    let file = ws.join("acme.code-workspace");
    std::fs::write(
        &file,
        format!(r#"{{ "folders": [{}] }}"#, folders.join(", ")),
    )
    .unwrap();
    file
}

#[test]
fn the_workspace_folder_and_its_file_name_the_workspace_from_anywhere() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let file = workspace(&root, &["api", "web"]);
    let folder = file.parent().unwrap();
    init(&data, folder);
    let ok = |o: &Output| {
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(out.contains("api") && out.contains("web"), "{out}");
    };
    // The folder init ran in holds no repo, only the workspace file and cartograph.yaml.
    ok(&cartograph(&data, &["status", folder.to_str().unwrap()]));
    // --workspace wins over [dir], here a folder no workspace covers.
    let elsewhere = root.join("data");
    ok(&cartograph(
        &data,
        &[
            "status",
            "--workspace",
            file.to_str().unwrap(),
            elsewhere.to_str().unwrap(),
        ],
    ));
    let o = cartograph(
        &data,
        &[
            "sync",
            "--workspace",
            file.to_str().unwrap(),
            elsewhere.to_str().unwrap(),
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn check_exits_3_for_a_repo_never_indexed_and_0_for_one_with_no_commits() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let data = root.join("data");
    let file = workspace(&root, &["api"]);
    let folder = file.parent().unwrap().to_owned();
    init(&data, &folder);
    // web joins the workspace file and the registry, but nothing has synced it yet.
    workspace(&root, &["api", "web"]);
    let plan = cartograph_discovery::setup::plan(&folder, None).unwrap();
    cartograph_discovery::setup::apply(&plan, &data).unwrap();
    let o = cartograph(&data, &["status", "--check", folder.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "the state says behind, so --check fails: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    // A repo with no commits has nothing to index: after a sync the workspace is current, not
    // behind forever (which made the app start a sync every 2 s).
    std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(folder.join("empty"))
        .status()
        .unwrap();
    std::fs::write(
        &file,
        r#"{ "folders": [{ "path": "api" }, { "path": "web" }, { "path": "empty" }] }"#,
    )
    .unwrap();
    let plan = cartograph_discovery::setup::plan(&folder, None).unwrap();
    cartograph_discovery::setup::apply(&plan, &data).unwrap();
    let o = cartograph(&data, &["sync", folder.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = cartograph(&data, &["status", "--check", folder.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
}

#[test]
fn a_json_error_is_one_object_on_stdout() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["status", "--json", d.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
    assert_eq!(v["schema"], 1);
    assert_eq!(v["error"]["code"], "usage");
    assert!(
        v["error"]["what"]
            .as_str()
            .unwrap()
            .contains("no workspace covers")
    );
    assert!(v["error"]["try"].as_str().is_some());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("error:"),
        "the human lines stay on stderr"
    );
}

#[test]
fn line_exits_0_with_no_workspace_and_plain_status_exits_2() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["status", "--line", d.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let line = String::from_utf8_lossy(&o.stdout);
    assert!(line.starts_with("cartograph: no workspace here") && line.len() < 120);
    let o = cartograph(d.path(), &["status", d.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
}
