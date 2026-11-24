//! SRV-24: `cartograph doctor`. Each check gets a fixture that fails it; running the command it
//! prints (or, where that touches the real machine, doing what the command does) makes it pass.
//! Each run gets its own HOME, CARTOGRAPH_HOME and a PATH holding only git and cartograph, so the
//! agents on the machine running the tests don't leak in.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cartograph_git::fixture::Fixture;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_cartograph");

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
    /// A folder with links to git and cartograph only.
    bin: PathBuf,
}

fn which(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{name} isn't on PATH"))
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (home, data, bin) = (root.join("home"), root.join("data"), root.join("bin"));
        for d in [&home, &data, &bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::os::unix::fs::symlink(which("git"), bin.join("git")).unwrap();
        std::os::unix::fs::symlink(BIN, bin.join("cartograph")).unwrap();
        Self {
            _tmp: tmp,
            root,
            home,
            data,
            bin,
        }
    }

    fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.env("HOME", &self.home)
            .env("CARTOGRAPH_HOME", &self.data)
            .env("PATH", &self.bin)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(BIN).args(args).output().unwrap()
    }

    /// `doctor --json` in `dir`: the exit code and each check by id.
    fn doctor(&self, dir: &Path) -> (i32, Vec<Value>) {
        let o = self.run(&["doctor", "--json", dir.to_str().unwrap()]);
        let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        });
        (
            o.status.code().unwrap(),
            v["checks"].as_array().unwrap().clone(),
        )
    }

    fn check(&self, dir: &Path, id: &str) -> Value {
        let (_, checks) = self.doctor(dir);
        checks
            .into_iter()
            .find(|c| c["id"].as_str().unwrap().starts_with(id))
            .unwrap_or_else(|| panic!("no check {id}"))
    }

    /// Runs a printed `cartograph …` command as a person would.
    fn fix(&self, cmd: &str) {
        let o = self.cmd("/bin/sh").args(["-c", cmd]).output().unwrap();
        assert!(
            o.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    /// Registers the folder as `cartograph init` does without a terminal (no index).
    fn register(&self, dir: &Path, file: Option<&Path>) {
        let plan = cartograph_discovery::setup::plan(dir, file).unwrap();
        cartograph_discovery::setup::apply(&plan, &self.data).unwrap();
    }
}

fn repo(dir: &Path) -> Fixture {
    let mut f = Fixture::init(dir);
    f.write("a.ts", "export {};\n");
    f.commit("one");
    f
}

fn fix_of(c: &Value) -> String {
    c["fix"].as_str().unwrap().to_owned()
}

#[test]
fn git_missing_fails_until_git_is_on_path() {
    let e = Env::new();
    let without = e.root.join("empty");
    std::fs::create_dir(&without).unwrap();
    let o = Command::new(BIN)
        .args(["doctor", "--json"])
        .current_dir(&e.root)
        .env("HOME", &e.home)
        .env("CARTOGRAPH_HOME", &e.data)
        .env("PATH", &without)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "a failed check exits 1");
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let git = &v["checks"][0];
    assert_eq!(git["id"], "git");
    assert_eq!(git["status"], "fail");
    assert_eq!(git["fix"], "install git 2.48 or later");
    // Installed: on PATH.
    assert_eq!(e.check(&e.root, "git")["status"], "pass");
}

#[test]
fn a_tilde_in_the_workspace_file_warns_until_it_is_relative() {
    let e = Env::new();
    let ws = e.root.join("ws");
    repo(&ws.join("api"));
    let file = ws.join("shop.code-workspace");
    std::fs::write(&file, r#"{"folders":[{"path":"api"},{"path":"~/web"}]}"#).unwrap();
    e.register(&ws, Some(&file));
    let c = e.check(&ws, "workspace");
    assert_eq!(c["status"], "warn", "{c}");
    assert!(c["why"].as_str().unwrap().contains("~/web"));
    assert!(fix_of(&c).contains("shop.code-workspace"));
    // What the line says to do: the path written relative to the file.
    std::fs::write(&file, r#"{"folders":[{"path":"api"},{"path":"../web"}]}"#).unwrap();
    assert_eq!(e.check(&ws, "workspace")["status"], "pass");
}

#[test]
fn a_missing_index_fails_until_sync_builds_it() {
    let e = Env::new();
    let ws = e.root.join("ws");
    repo(&ws.join("api"));
    e.register(&ws, None);
    let c = e.check(&ws, "index");
    assert_eq!(c["status"], "fail", "{c}");
    assert_eq!(fix_of(&c), format!("cartograph sync {}", ws.display()));
    e.fix(&fix_of(&c));
    assert_eq!(e.check(&ws, "index")["status"], "pass");
}

#[test]
fn a_synced_data_folder_fails_until_it_moves() {
    let e = Env::new();
    let synced = e.root.join("Dropbox/carto");
    let o = e
        .cmd(BIN)
        .args(["doctor", "--json", e.root.to_str().unwrap()])
        .env("CARTOGRAPH_HOME", &synced)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let c = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "index")
        .unwrap();
    assert_eq!(c["status"], "fail");
    assert!(fix_of(c).contains("CARTOGRAPH_HOME"), "{c}");
    // Moved out, CARTOGRAPH_HOME pointing at it (Env's data folder).
    assert_eq!(e.check(&e.root, "index")["status"], "pass");
}

#[test]
fn a_repo_that_is_gone_fails_until_init_reads_the_workspace_again() {
    let e = Env::new();
    let ws = e.root.join("ws");
    repo(&ws.join("api"));
    repo(&ws.join("web"));
    e.register(&ws, None);
    std::fs::remove_dir_all(ws.join("web")).unwrap();
    let (code, checks) = e.doctor(&ws);
    assert_eq!(code, 1);
    let c = checks.iter().find(|c| c["id"] == "repo web").unwrap();
    assert_eq!(c["status"], "fail");
    e.fix(&fix_of(c));
    let (_, checks) = e.doctor(&ws);
    assert!(checks.iter().all(|c| c["id"] != "repo web"));
    assert_eq!(
        checks.iter().find(|c| c["id"] == "repo api").unwrap()["status"],
        "pass"
    );
}

#[test]
fn a_shallow_clone_warns_until_unshallowed() {
    let e = Env::new();
    let mut origin = repo(&e.root.join("origin"));
    origin.write("b.ts", "export {};\n");
    origin.commit("two");
    let ws = e.root.join("ws");
    std::fs::create_dir(&ws).unwrap();
    let url = format!("file://{}", origin.dir.display());
    let clone = ws.join("api");
    let o = e
        .cmd("git")
        .args(["clone", "-q", "--depth", "1", &url, clone.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    e.register(&ws, None);
    let c = e.check(&ws, "repo api");
    assert_eq!(c["status"], "warn", "{c}");
    assert_eq!(
        fix_of(&c),
        format!("git -C {} fetch --unshallow", clone.display())
    );
    e.fix(&fix_of(&c));
    assert_eq!(e.check(&ws, "repo api")["status"], "pass");
}

#[test]
fn a_hook_running_a_gone_binary_fails_and_fix_reinstalls_it() {
    let e = Env::new();
    let ws = e.root.join("ws");
    let f = repo(&ws.join("api"));
    e.register(&ws, None);
    // Where `hooks install` would have put it with an older binary: git config from 2.54, the
    // hook file before.
    let stale = "'/gone/cartograph' sync --quiet --background --hook-args=\"$*\"";
    let v = f.git(&["--version"]);
    let mut it = v.split_whitespace().nth(2).unwrap().split('.');
    let (major, minor): (u32, u32) = (
        it.next().unwrap().parse().unwrap(),
        it.next().unwrap().parse().unwrap(),
    );
    if (major, minor) >= (2, 54) {
        f.git(&["config", "--local", "hook.cartograph.command", stale]);
    } else {
        std::fs::write(
            f.dir.join(".git/hooks/post-commit"),
            format!("#!/bin/sh\n# cartograph:start\n{stale}\n# cartograph:end\n"),
        )
        .unwrap();
    }
    let c = e.check(&ws, "hooks");
    assert_eq!(c["status"], "fail", "{c}");
    assert_eq!(
        fix_of(&c),
        format!("cartograph hooks install {}", ws.display())
    );
    assert_eq!(c["auto_fix"], true);
    // --fix runs it, because `hooks remove` undoes it.
    let o = e.run(&["doctor", "--fix", "--json", ws.to_str().unwrap()]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let c = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "hooks")
        .unwrap()
        .clone();
    assert_eq!(c["status"], "pass", "{c}");
    assert_eq!(c["fixed"], true);
}

/// `schedule hourly` itself would load a real launchd agent or systemd timer, so the test writes
/// the job file the way that command does, with this binary's path.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_schedule_running_a_gone_binary_fails() {
    let e = Env::new();
    let write = |exe: &str| {
        if cfg!(target_os = "macos") {
            let d = e.home.join("Library/LaunchAgents");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("dev.cartograph.sync.plist"),
                format!("<plist>\n  <array>\n    <string>{exe}</string>\n    <string>sync</string>\n  </array>\n</plist>\n"),
            )
            .unwrap();
        } else {
            let d = e.home.join(".config/systemd/user");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("cartograph-sync.service"),
                format!("[Service]\nExecStart='{exe}' sync --fetch --quiet --all-workspaces\n"),
            )
            .unwrap();
        }
    };
    write("/gone/cartograph");
    let c = e.check(&e.root, "schedule");
    assert_eq!(c["status"], "fail", "{c}");
    assert_eq!(fix_of(&c), "cartograph schedule hourly");
    write(BIN);
    assert_eq!(e.check(&e.root, "schedule")["status"], "pass");
}

#[test]
fn an_agent_not_set_up_warns_until_agents_install() {
    let e = Env::new();
    std::fs::create_dir(e.home.join(".cursor")).unwrap();
    let c = e.check(&e.root, "agents");
    assert_eq!(c["status"], "warn", "{c}");
    assert!(c["why"].as_str().unwrap().contains("Cursor"));
    assert_eq!(fix_of(&c), "cartograph agents install");
    assert_eq!(c["auto_fix"], false, "it needs the preview");
    e.fix(&format!("{} --yes", fix_of(&c)));
    assert_eq!(e.check(&e.root, "agents")["status"], "pass");
}

#[test]
fn two_clones_of_one_repo_warn_until_one_is_left() {
    let e = Env::new();
    let origin = repo(&e.root.join("origin"));
    let ws = e.root.join("ws");
    std::fs::create_dir(&ws).unwrap();
    for name in ["a", "b"] {
        let o = e
            .cmd("git")
            .args([
                "clone",
                "-q",
                origin.dir.to_str().unwrap(),
                ws.join(name).to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(o.status.success());
    }
    e.register(&ws, None);
    let c = e.check(&ws, "clones");
    assert_eq!(c["status"], "warn", "{c}");
    let fix = fix_of(&c);
    assert!(
        fix.contains(&format!("cartograph init {}", ws.display())),
        "{fix}"
    );
    std::fs::remove_dir_all(ws.join("b")).unwrap();
    e.fix(&format!("cartograph init {}", ws.display()));
    assert_eq!(e.check(&ws, "clones")["status"], "pass");
}

#[test]
fn the_human_output_is_one_line_per_check() {
    let e = Env::new();
    let o = e.run(&["doctor", e.root.to_str().unwrap()]);
    let text = String::from_utf8(o.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // git, workspace, index, hooks, schedule, agents, clones: no repos without a workspace.
    assert_eq!(lines.len(), 7, "{text}");
    assert!(lines[0].starts_with("ok   git: git "));
    assert!(lines[1].starts_with("warn workspace: no workspace covers "));
    assert!(lines[1].contains(" · fix: cartograph init "));
}
