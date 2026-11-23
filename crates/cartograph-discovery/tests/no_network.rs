//! IDX-09: the default-branch ladder resolves with the network denied to the process (macOS
//! sandbox-exec, Linux unshare). A repo whose origin is a real https URL proves no rung reaches it.

use std::path::Path;
use std::process::Command;

use cartograph_discovery::{Source, default_branch};

const REPO: &str = "CARTOGRAPH_NO_NETWORK_REPO";

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
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
        .status()
        .expect("git runs")
        .success();
    assert!(ok, "git {args:?}");
}

/// Runs inside the sandbox, started by the test below.
#[test]
#[ignore = "started by the_ladder_runs_with_the_network_denied"]
fn ladder_inside_the_sandbox() {
    let repo = std::env::var_os(REPO).expect("started by the outer test");
    let (branch, source) = default_branch(Path::new(&repo), None).unwrap();
    assert_eq!((branch.as_str(), source), ("trunk", Source::OriginHead));
    let (remote, root) = cartograph_discovery::registry::repo_identity(Path::new(&repo));
    assert_eq!(
        remote.as_deref(),
        Some("https://github.com/ax3cubed/cartograph.git")
    );
    assert!(root.is_some());
}

#[test]
fn the_ladder_runs_with_the_network_denied() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("r");
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "work"]);
    std::fs::write(d.join("a"), "x").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "a"]);
    git(
        &d,
        &[
            "remote",
            "add",
            "origin",
            "https://u:secret@github.com/ax3cubed/cartograph.git",
        ],
    );
    git(&d, &["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
    git(
        &d,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );

    let exe = std::env::current_exe().unwrap();
    let inner = [
        "--exact",
        "ladder_inside_the_sandbox",
        "--ignored",
        "--nocapture",
    ];
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/sandbox-exec");
        c.args(["-p", "(version 1)(allow default)(deny network*)"])
            .arg(&exe);
        c
    } else if cfg!(target_os = "linux")
        && Command::new("unshare")
            .args(["-rn", "true"])
            .status()
            .is_ok_and(|s| s.success())
    {
        let mut c = Command::new("unshare");
        c.arg("-rn").arg(&exe);
        c
    } else {
        eprintln!("skipped: no way to deny the network to a process on this machine");
        return;
    };
    let out = cmd.args(inner).env(REPO, &d).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("sandbox_apply") {
        // Already inside a sandbox that forbids nesting (some CI runners and agent shells).
        eprintln!("skipped: {stderr}");
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("1 passed"), "{stdout}");
}
