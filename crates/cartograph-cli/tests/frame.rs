//! The CLI frame (SRV-01): help, one golden per error class, exit codes, JSON and escaping.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use cartograph_git::fixture::Fixture;

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn fixture_repo(root: &Path) -> Fixture {
    let mut f = Fixture::init(&root.join("repo"));
    f.write("a.ts", "import './b';");
    f.write("b.ts", "");
    f.commit("one");
    f
}

#[test]
fn help_lists_every_command() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["--help"]);
    assert_eq!(o.status.code(), Some(0));
    insta::assert_snapshot!(text(&o.stdout));
}

#[test]
fn error_class_usage_exits_2() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["status", "--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    insta::assert_snapshot!(text(&o.stderr));
}

#[test]
fn error_class_not_built_exits_1() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["report"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    insta::assert_snapshot!(text(&o.stderr));
}

#[test]
fn error_class_failure_exits_1() {
    // A crate error rendered through Explain: no HOME and no CARTOGRAPH_HOME.
    let o = Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .arg("paths")
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    insta::assert_snapshot!(text(&o.stderr));
}

#[test]
fn error_class_needs_yes_exits_2_and_yes_rebuilds() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let f = fixture_repo(d.path());
    let ix = d.path().join("index.cartograph");
    let repo = f.dir.to_str().unwrap();
    let first = cartograph(&home, &["index", repo, "--out", ix.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");

    let refused = cartograph(
        &home,
        &["index", repo, "--rebuild", "--out", ix.to_str().unwrap()],
    );
    assert_eq!(refused.status.code(), Some(2));
    let err = text(&refused.stderr).replace(d.path().to_str().unwrap(), "<tmp>");
    insta::assert_snapshot!(err);

    let yes = cartograph(
        &home,
        &[
            "index",
            repo,
            "--rebuild",
            "--yes",
            "--out",
            ix.to_str().unwrap(),
        ],
    );
    assert_eq!(yes.status.code(), Some(0), "{yes:?}");
    assert!(ix.exists() && !ix.with_extension("cartograph.new").exists());
}

#[test]
fn json_carries_schema_1() {
    let d = tempfile::tempdir().unwrap();
    // Plain `status` exits 2 with no workspace (spec exit codes); `--line` always answers.
    let o = cartograph(d.path(), &["status", "--line", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["schema"], 1);
}

#[test]
fn status_line_is_one_short_line_and_exits_0_without_an_index() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["status", "--line", d.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let out = text(&o.stdout);
    assert_eq!(out.lines().count(), 1);
    assert!(out.len() < 120);
}

#[test]
fn quiet_prints_nothing_on_success() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["status", "--line", "--quiet"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stdout.is_empty() && o.stderr.is_empty());
}

#[test]
fn escapes_in_a_path_never_reach_the_terminal() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let f = fixture_repo(d.path());
    // Clears the screen and sets the window title, if printed raw.
    let ix = d.path().join("x\x1b[2J\x1b]0;pwned\x07.cartograph");
    let o = cartograph(
        &home,
        &[
            "index",
            f.dir.to_str().unwrap(),
            "--out",
            ix.to_str().unwrap(),
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let out = text(&o.stdout);
    assert!(out.contains("x[2J]0;pwned.cartograph"), "{out}");
    assert!(!out.chars().any(|c| c.is_control() && c != '\n'), "{out:?}");

    // And on stderr: the second run fails because the file exists, naming the path.
    let o = cartograph(
        &home,
        &[
            "index",
            f.dir.to_str().unwrap(),
            "--out",
            ix.to_str().unwrap(),
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(
        err.contains("pwned") && !err.contains('\x1b') && !err.contains('\x07'),
        "{err:?}"
    );
}

/// Every workspace command takes the folder as its last positional argument (clig.dev); `--dir`
/// still works, hidden, and naming the folder both ways is a usage error.
#[test]
fn every_command_takes_dir_last_and_dir_flag_stays_hidden() {
    let d = tempfile::tempdir().unwrap();
    for cmd in [
        "init", "sync", "status", "verify", "open", "ask", "impact", "tool", "doctor",
    ] {
        let o = cartograph(d.path(), &[cmd, "--help"]);
        let help = text(&o.stdout);
        assert!(help.contains("[DIR]"), "{cmd}: {help}");
        assert!(!help.contains("--dir"), "{cmd}: {help}");
    }
    insta::assert_snapshot!(text(&cartograph(d.path(), &["verify", "--help"]).stdout));
    let f = fixture_repo(d.path());
    let repo = f.dir.to_str().unwrap();
    // No init here, so each form reaches the workspace lookup, which names the folder it was given.
    for args in [&["status", repo][..], &["status", "--dir", repo]] {
        let err = text(&cartograph(d.path(), args).stderr);
        assert!(
            err.contains(&format!("no workspace covers {repo}")),
            "{args:?}: {err}"
        );
    }
    let both = cartograph(d.path(), &["status", repo, "--dir", repo]);
    assert_eq!(both.status.code(), Some(2));
    assert!(
        text(&both.stderr).contains("cannot be used with"),
        "{}",
        text(&both.stderr)
    );
}

#[test]
fn background_sync_refuses_flags_it_cannot_honour() {
    // The background child is a plain `sync --quiet`: a --fetch or --wait asked for alongside it
    // was silently dropped, and the caller was told the sync had started.
    let d = tempfile::tempdir().unwrap();
    for flag in ["--fetch", "--wait"] {
        let o = cartograph(d.path(), &["sync", "--background", flag]);
        assert_eq!(o.status.code(), Some(2), "{flag}");
        assert!(
            text(&o.stderr).contains("cannot be used with"),
            "{flag}: {}",
            text(&o.stderr)
        );
    }
}
