//! `cartograph init` (IDX-08 to IDX-11): manifest, registry, user-only data folder, the same index
//! from any subfolder, and a synced data folder refused.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cartograph_git::fixture::Fixture;
use serde_json::Value;

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn json(o: &Output) -> Value {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}

/// `acme/` (not a repo) with `api` and `web` repos and a workspace file that also lists `notes`.
fn workspace(root: &Path) -> PathBuf {
    let acme = root.join("acme");
    for (name, file) in [("api", "src/server.ts"), ("web", "packages/ui/index.ts")] {
        let mut f = Fixture::init(&acme.join(name));
        f.write(file, "export const x = 1;\n");
        f.commit("first");
    }
    std::fs::create_dir_all(acme.join("notes")).unwrap();
    std::fs::write(
        acme.join("acme.code-workspace"),
        r#"{ "folders": [ { "path": "api" }, { "path": "web/packages/ui", "name": "UI" }, { "path": "notes" }, ] }"#,
    )
    .unwrap();
    std::fs::canonicalize(acme).unwrap()
}

#[test]
fn init_logs_its_first_index_like_sync() {
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let home = t.path().join("home");
    let o = cartograph(
        &home,
        &[
            "init",
            &acme.display().to_string(),
            "--yes",
            "--json",
            "-vv",
        ],
    );
    let id = json(&o)["workspace"].as_str().unwrap().to_owned();
    // The workspace's log file exists, as for `cartograph sync`: with no subscriber, init's
    // warnings (a resumed run, a checkpoint that fell back) went nowhere.
    let logs = home.join("workspaces").join(&id).join("logs");
    let files: Vec<_> = std::fs::read_dir(&logs)
        .unwrap_or_else(|e| panic!("{}: {e}", logs.display()))
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("cartograph"))
        .collect();
    assert!(!files.is_empty(), "no log file in {}", logs.display());
}

#[test]
fn init_writes_the_manifest_registry_and_a_user_only_index() {
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let home = t.path().join("home");
    let a = acme.display().to_string();

    let first = json(&cartograph(&home, &["init", &a, "--yes", "--json"]));
    let id = first["workspace"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 8);
    assert_eq!(first["skipped"][0]["reason"], "not-a-git-repository");
    assert_eq!(first["repos"][1]["scopes"][0], "packages/ui");
    let index = PathBuf::from(first["index"].as_str().unwrap());
    assert!(index.is_file());
    let manifest = std::fs::read_to_string(acme.join("cartograph.yaml")).unwrap();
    assert!(manifest.contains("default_branch: main"), "{manifest}");
    assert!(home.join("registry.toml").is_file());

    // The phase 2 exit gate: no group- or world-readable file in the data folder.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fn walk(p: &Path, bad: &mut Vec<String>) {
            let m = std::fs::symlink_metadata(p).unwrap();
            if m.permissions().mode() & 0o077 != 0 {
                bad.push(format!(
                    "{} {:o}",
                    p.display(),
                    m.permissions().mode() & 0o777
                ));
            }
            if m.is_dir() {
                for e in std::fs::read_dir(p).unwrap() {
                    walk(&e.unwrap().path(), bad);
                }
            }
        }
        let mut bad = Vec::new();
        walk(&home, &mut bad);
        assert!(bad.is_empty(), "{bad:?}");
    }

    // Any subfolder of any repo in the workspace finds the same index.
    std::fs::create_dir_all(acme.join("web/packages/ui/deep")).unwrap();
    for sub in ["api/src", "web/packages/ui/deep", "web"] {
        let found = cartograph_discovery::setup::locate(&home, &acme.join(sub))
            .unwrap()
            .unwrap();
        assert_eq!(found.index, index, "{sub}");
    }

    // Running it again keeps the workspace id and the manifest.
    let again = json(&cartograph(&home, &["init", &a, "--no-index", "--json"]));
    assert_eq!(again["workspace"], id.as_str());
    assert_eq!(
        std::fs::read_to_string(acme.join("cartograph.yaml")).unwrap(),
        manifest
    );
}

/// The manifest is committed: it names no path on this machine, and a teammate's init (another
/// data folder, a hand-written comment, the same repos) leaves it byte for byte.
#[test]
fn a_teammate_s_init_leaves_the_committed_manifest_alone() {
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let a = acme.display().to_string();
    json(&cartograph(
        &t.path().join("mine"),
        &["init", &a, "--no-index", "--json"],
    ));
    let path = acme.join("cartograph.yaml");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(!written.contains("yaml-language-server"), "{written}");
    assert!(
        !written.contains(&t.path().display().to_string()),
        "{written}"
    );
    let edited = format!("# Our workspace.\n{written}");
    std::fs::write(&path, &edited).unwrap();
    json(&cartograph(
        &t.path().join("theirs"),
        &["init", &a, "--no-index", "--json"],
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}

/// A teammate who hasn't cloned every repo merges into the committed manifest: repos missing
/// here, hand-added excludes and services stay; only a newly discovered repo is added.
#[test]
fn init_merges_into_the_committed_manifest() {
    use cartograph_discovery::manifest::{self, Service};
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let a = acme.display().to_string();
    json(&cartograph(
        &t.path().join("mine"),
        &["init", &a, "--no-index", "--json"],
    ));
    let path = acme.join("cartograph.yaml");
    let mut m = manifest::read(&path).unwrap().manifest;
    m.options.exclude.push("**/generated".into());
    m.services.push(Service {
        name: "ui".into(),
        repo: "web".into(),
        path: Some("packages/ui".into()),
        tags: vec![],
    });
    std::fs::write(&path, manifest::to_yaml(&m)).unwrap();
    let committed = std::fs::read_to_string(&path).unwrap();

    // The teammate has api only.
    std::fs::rename(acme.join("web"), t.path().join("web-elsewhere")).unwrap();
    json(&cartograph(
        &t.path().join("theirs"),
        &["init", &a, "--no-index", "--json"],
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        committed,
        "nothing changed"
    );

    // A repo the workspace file now lists is added; everything else is kept.
    let mut extra = Fixture::init(&acme.join("extra"));
    extra.write("x.ts", "export {};\n");
    extra.commit("first");
    std::fs::write(
        acme.join("acme.code-workspace"),
        r#"{ "folders": [ { "path": "api" }, { "path": "web/packages/ui", "name": "UI" }, { "path": "notes" }, { "path": "extra" } ] }"#,
    )
    .unwrap();
    json(&cartograph(
        &t.path().join("theirs"),
        &["init", &a, "--no-index", "--json"],
    ));
    let merged = manifest::read(&path).unwrap().manifest;
    let names: Vec<&str> = merged.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["api", "web", "extra"]);
    assert_eq!(merged.services, m.services);
    assert!(merged.options.exclude.contains(&"**/generated".to_owned()));
    assert_eq!(merged.repos[..2], m.repos[..]);
}

#[test]
fn without_a_terminal_or_yes_only_the_manifest_is_written() {
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let home = t.path().join("home");
    let o = json(&cartograph(
        &home,
        &["init", &acme.display().to_string(), "--json"],
    ));
    assert_eq!(o["index"], Value::Null);
    assert!(acme.join("cartograph.yaml").is_file());
    assert!(
        !PathBuf::from(&home)
            .join("workspaces")
            .join(o["workspace"].as_str().unwrap())
            .join("index.cartograph")
            .exists()
    );
}

#[test]
fn a_data_folder_inside_dropbox_is_refused_with_the_reason() {
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let o = cartograph(
        &t.path().join("Dropbox/c"),
        &["init", &acme.display().to_string(), "--yes"],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("sync client"), "{err}");
    assert!(!t.path().join("Dropbox/c").exists());
}

#[test]
fn no_workspace_is_a_usage_error() {
    let t = tempfile::tempdir().unwrap();
    let o = cartograph(
        &t.path().join("home"),
        &["init", &t.path().display().to_string()],
    );
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn init_indexes_each_repo_under_its_own_name_whatever_the_manifest_order() {
    let t = tempfile::tempdir().unwrap();
    let acme = t.path().join("acme");
    for (name, file) in [("api", "src/api.ts"), ("web", "src/web.ts")] {
        let mut f = Fixture::init(&acme.join(name));
        f.write(file, "export const x = 1;\n");
        f.commit("first");
    }
    std::fs::write(
        acme.join("acme.code-workspace"),
        r#"{ "folders": [ { "path": "api" }, { "path": "web" } ] }"#,
    )
    .unwrap();
    // The committed manifest lists web first; discovery finds the repos sorted by folder. Paired by
    // position, api's files were indexed under the name web and web's under api.
    std::fs::write(
        acme.join("cartograph.yaml"),
        "version: 1\nworkspace:\n  name: acme\nrepos:\n- name: web\n  path: web\n- name: api\n  path: api\n",
    )
    .unwrap();
    let acme = std::fs::canonicalize(acme).unwrap();
    let home = t.path().join("home");
    let first = json(&cartograph(
        &home,
        &["init", &acme.display().to_string(), "--yes", "--json"],
    ));
    let index = PathBuf::from(first["index"].as_str().unwrap());
    let c = cartograph_store::rusqlite::Connection::open(&index).unwrap();
    let mut live: Vec<String> = c
        .prepare("SELECT path FROM file_path WHERE to_seq IS NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    live.sort();
    assert_eq!(live, ["api/src/api.ts", "web/src/web.ts"]);
}

/// The ladder (IDX-09) picks a branch only for a repo the manifest doesn't have yet: a committed
/// `default_branch`, or a committed entry without one, is never changed by an init.
#[test]
fn init_never_changes_a_committed_default_branch() {
    use cartograph_discovery::manifest;
    let t = tempfile::tempdir().unwrap();
    let acme = workspace(t.path());
    let a = acme.display().to_string();
    json(&cartograph(
        &t.path().join("mine"),
        &["init", &a, "--no-index", "--json"],
    ));
    let path = acme.join("cartograph.yaml");
    // The team indexes api's `develop`, where the ladder would say main, and leaves web unpinned.
    Fixture::init(&acme.join("api")).git(&["branch", "develop"]);
    let mut m = manifest::read(&path).unwrap().manifest;
    for r in &mut m.repos {
        r.default_branch = (r.name == "api").then(|| "develop".into());
    }
    std::fs::write(&path, manifest::to_yaml(&m)).unwrap();
    let committed = std::fs::read_to_string(&path).unwrap();
    json(&cartograph(
        &t.path().join("theirs"),
        &["init", &a, "--no-index", "--json"],
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), committed);

    // A repo added later gets its branch from the ladder; the committed ones still don't change.
    let mut extra = Fixture::init(&acme.join("extra"));
    extra.write("x.ts", "export {};\n");
    extra.commit("first");
    std::fs::write(
        acme.join("acme.code-workspace"),
        r#"{ "folders": [ { "path": "api" }, { "path": "web/packages/ui", "name": "UI" }, { "path": "extra" } ] }"#,
    )
    .unwrap();
    json(&cartograph(
        &t.path().join("theirs"),
        &["init", &a, "--no-index", "--json"],
    ));
    let merged = manifest::read(&path).unwrap().manifest;
    let branches: Vec<(&str, Option<&str>)> = merged
        .repos
        .iter()
        .map(|r| (r.name.as_str(), r.default_branch.as_deref()))
        .collect();
    assert_eq!(
        branches,
        [
            ("api", Some("develop")),
            ("web", None),
            ("extra", Some("main"))
        ]
    );
}
