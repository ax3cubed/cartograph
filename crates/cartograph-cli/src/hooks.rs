//! `cartograph hooks install | remove | status` (IDX-26, D3): git hooks that ask for a background
//! sync after each commit, merge, checkout and rewrite, in every repo of the workspace.
//!
//! - Git 2.54 or later: `hook.cartograph.command` and one `hook.cartograph.event` per event in the
//!   repo's own config. They run beside any other hooks.
//! - Older git: the hooks folder from `git rev-parse --git-path hooks` (so `core.hooksPath` is
//!   honoured). A missing hook is created; an existing shell hook gets a block between
//!   `# cartograph:start` and `# cartograph:end` appended; anything else (a symlink, a binary, a
//!   script in another language) is refused and the line printed.
//! - Husky or lefthook: nothing is written; the snippet is printed for their config.
//!
//! The line runs the absolute path of this binary, quoted, and nothing from the repo. It is
//! `sync --quiet --background`, which does the D2 lock try and returns; a checkout of files and a
//! rebase in progress return before even that. Removing restores the hook's exact previous bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

use cartograph_core::{ExitCode, Report};
use serde_json::json;

use crate::output::Out;

pub const EVENTS: [&str; 4] = ["post-commit", "post-merge", "post-checkout", "post-rewrite"];
const START: &str = "# cartograph:start";
const END: &str = "# cartograph:end";

/// Git's version as (major, minor).
pub fn git_version() -> Option<(u32, u32)> {
    let out = Command::new("git").arg("--version").output().ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s.split_whitespace().nth(2)?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Config-based hooks arrived in git 2.54.
pub fn config_hooks(v: (u32, u32)) -> bool {
    v >= (2, 54)
}

/// `'…'`, with `'` written as `'\''`: the binary's path as one shell word.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The command a hook runs: this binary, quoted, and fixed words. `$*` carries the hook's
/// arguments, so a file checkout (post-checkout's third argument 0) returns at once.
pub fn hook_line(exe: &Path) -> String {
    format!(
        "{} sync --quiet --background --hook-args=\"$*\"",
        quote(&exe.to_string_lossy())
    )
}

fn block(line: &str) -> String {
    format!("\n{START}\n{line}\n{END}\n")
}

/// The `git config` calls of a config-based install, as argument lists (git 2.54+).
pub fn config_install_args(line: &str) -> Vec<Vec<String>> {
    let mut v = vec![vec![
        "config".into(),
        "--local".into(),
        "hook.cartograph.command".into(),
        line.into(),
    ]];
    v.push(vec![
        "config".into(),
        "--local".into(),
        "--unset-all".into(),
        "hook.cartograph.event".into(),
    ]);
    for e in EVENTS {
        v.push(vec![
            "config".into(),
            "--local".into(),
            "--add".into(),
            "hook.cartograph.event".into(),
            e.into(),
        ]);
    }
    v
}

/// A hook manager that owns the hooks, if any: then nothing is written.
pub fn manager(repo: &Path) -> Option<&'static str> {
    if repo.join(".husky").is_dir() {
        return Some("husky");
    }
    [
        "lefthook.yml",
        "lefthook.yaml",
        ".lefthook.yml",
        ".lefthook.yaml",
    ]
    .iter()
    .any(|f| repo.join(f).is_file())
    .then_some("lefthook")
}

/// The hooks folder (`git rev-parse --git-path hooks`, which honours core.hooksPath).
pub(crate) fn hooks_dir(repo: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    out.status
        .success()
        .then(|| if p.is_absolute() { p } else { repo.join(p) })
}

fn git(repo: &Path, args: &[String]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// What happened to one hook file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    Created,
    Appended,
    AlreadyThere,
    Removed,
    Restored,
    Absent,
    /// Not a regular shell hook: nothing written.
    Refused(String),
}

/// Installs the block in one hook file. Only a regular file inside the folder is edited, never
/// through a symlink.
pub fn install_file(path: &Path, line: &str) -> std::io::Result<FileOutcome> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_hook(path, &format!("#!/bin/sh{}", block(line)))?;
            return Ok(FileOutcome::Created);
        }
        Err(e) => return Err(e),
    };
    if !meta.file_type().is_file() {
        return Ok(FileOutcome::Refused("not a regular file".into()));
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Ok(FileOutcome::Refused("not a text file".into())),
    };
    let first = text.lines().next().unwrap_or("");
    let shell = first.starts_with("#!")
        && ["sh", "bash", "zsh", "dash", "ksh"]
            .iter()
            .any(|s| first.ends_with(&format!("/{s}")) || first.ends_with(&format!(" {s}")));
    if !shell {
        return Ok(FileOutcome::Refused("not a shell script".into()));
    }
    if let Some(stripped) = strip_block(&text) {
        let again = format!("{stripped}{}", block(line));
        if again == text {
            return Ok(FileOutcome::AlreadyThere);
        }
        write_hook(path, &again)?;
        return Ok(FileOutcome::AlreadyThere);
    }
    write_hook(path, &format!("{text}{}", block(line)))?;
    Ok(FileOutcome::Appended)
}

/// The text without our block, if it has one.
fn strip_block(text: &str) -> Option<String> {
    let start = text.find(&format!("\n{START}\n"))?;
    let end_at = text[start..].find(&format!("\n{END}\n"))? + start + END.len() + 2;
    Some(format!("{}{}", &text[..start], &text[end_at..]))
}

/// Removes the block from one hook file; a hook that held only our block is deleted, as it was
/// absent before.
pub fn remove_file(path: &Path) -> std::io::Result<FileOutcome> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(FileOutcome::Absent);
    };
    if !meta.file_type().is_file() {
        return Ok(FileOutcome::Absent);
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(FileOutcome::Absent);
    };
    let Some(rest) = strip_block(&text) else {
        return Ok(FileOutcome::Absent);
    };
    if rest == "#!/bin/sh" {
        std::fs::remove_file(path)?;
        return Ok(FileOutcome::Removed);
    }
    write_hook(path, &rest)?;
    Ok(FileOutcome::Restored)
}

fn write_hook(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    // Written in place, so the file keeps its own mode; a new one is made executable.
    let new = !path.exists();
    std::fs::write(path, text)?;
    #[cfg(unix)]
    if new {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = new;
    Ok(())
}

/// The line a hook file or the config holds, if ours is there.
pub(crate) fn installed_line(repo: &Path, hooks: Option<&Path>) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--local", "--get", "hook.cartograph.command"])
        .output()
        .ok()?;
    if out.status.success() {
        return Some(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    }
    let text = std::fs::read_to_string(hooks?.join(EVENTS[0])).ok()?;
    let start = text.find(&format!("{START}\n"))? + START.len() + 1;
    text[start..].lines().next().map(str::to_owned)
}

/// Runs install, remove or status over every repo of the workspace at `dir`.
pub fn run(out: &Out, dir: &Path, action: crate::Setup) -> Result<(), Report> {
    let data = crate::data_dir()?;
    let applied = cartograph_discovery::setup::locate(&data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    let ws = cartograph_server::workspace(&data, &applied.workspace_id).map_err(|p| Report {
        what: p.what,
        why: p.why,
        action: p.action,
        exit_code: ExitCode::Error,
    })?;
    let exe = std::env::current_exe()
        .map_err(|e| crate::io_report("can't find the cartograph binary".into(), e))?;
    let line = hook_line(&exe);
    let version = git_version().unwrap_or((0, 0));
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    for r in &ws.repos {
        let hooks = hooks_dir(&r.path);
        let (word, detail): (&str, String) = match action {
            crate::Setup::Install => {
                if let Some(m) = manager(&r.path) {
                    (
                        "manager",
                        format!(
                            "{m} owns the hooks; add to its post-commit, post-merge, post-checkout and post-rewrite: {line}"
                        ),
                    )
                } else if config_hooks(version) {
                    let ok = config_install_args(&line)
                        .iter()
                        .enumerate()
                        // --unset-all fails when there is nothing to unset.
                        .all(|(i, a)| git(&r.path, a) || i == 1);
                    if ok {
                        ("installed", "in the repo's git config".into())
                    } else {
                        ("failed", "git config failed".into())
                    }
                } else {
                    let Some(h) = &hooks else {
                        lines.push(format!("{}: not a git repository", r.name));
                        continue;
                    };
                    let mut refused = Vec::new();
                    for e in EVENTS {
                        match install_file(&h.join(e), &line) {
                            Ok(FileOutcome::Refused(why)) => refused.push(format!("{e} ({why})")),
                            Ok(_) => {}
                            Err(err) => refused.push(format!("{e} ({err})")),
                        }
                    }
                    if refused.is_empty() {
                        ("installed", h.display().to_string())
                    } else {
                        (
                            "refused",
                            format!(
                                "left as they are: {}; add this line to them: {line}",
                                refused.join(", ")
                            ),
                        )
                    }
                }
            }
            crate::Setup::Remove => {
                let _ = git(
                    &r.path,
                    &["config", "--local", "--remove-section", "hook.cartograph"].map(String::from),
                );
                if let Some(h) = &hooks {
                    for e in EVENTS {
                        let _ = remove_file(&h.join(e));
                    }
                }
                ("removed", String::new())
            }
            crate::Setup::Status => match installed_line(&r.path, hooks.as_deref()) {
                Some(l) if l == line => ("installed", String::new()),
                Some(l) => (
                    "stale",
                    format!("runs another binary: {l}; run `cartograph hooks install`"),
                ),
                None => (
                    "absent",
                    manager(&r.path)
                        .map(|m| format!("{m} owns the hooks"))
                        .unwrap_or_default(),
                ),
            },
        };
        lines.push(if detail.is_empty() {
            format!("{}: {word}", r.name)
        } else {
            format!("{}: {word} ({detail})", r.name)
        });
        rows.push(json!({ "repo": r.name, "state": word, "detail": detail }));
    }
    out.print(&lines.join("\n"), json!({ "repos": rows }));
    Ok(())
}

/// The hook entry point's early return (D3): a post-checkout that only checked out files (third
/// argument 0) or any hook during a rebase starts nothing.
pub fn skip(dir: &Path, hook_args: &str) -> bool {
    let args: Vec<&str> = hook_args.split_whitespace().collect();
    if args.len() == 3 && args[2] == "0" {
        return true;
    }
    let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "rev-parse",
            "--git-path",
            "rebase-merge",
            "--git-path",
            "rebase-apply",
        ])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).lines().any(|p| {
        let p = Path::new(p.trim());
        if p.is_absolute() {
            p.exists()
        } else {
            dir.join(p).exists()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_the_line() {
        assert_eq!(parse_version("git version 2.54.1\n"), Some((2, 54)));
        assert_eq!(
            parse_version("git version 2.39.3 (Apple Git-146)"),
            Some((2, 39))
        );
        assert!(config_hooks((2, 54)) && config_hooks((3, 0)) && !config_hooks((2, 53)));
        assert_eq!(
            hook_line(Path::new("/opt/it's here/cartograph")),
            "'/opt/it'\\''s here/cartograph' sync --quiet --background --hook-args=\"$*\""
        );
    }

    #[test]
    fn config_install_is_the_command_and_four_events() {
        let golden: Vec<String> = config_install_args("L")
            .iter()
            .map(|a| a.join(" "))
            .collect();
        assert_eq!(
            golden,
            [
                "config --local hook.cartograph.command L",
                "config --local --unset-all hook.cartograph.event",
                "config --local --add hook.cartograph.event post-commit",
                "config --local --add hook.cartograph.event post-merge",
                "config --local --add hook.cartograph.event post-checkout",
                "config --local --add hook.cartograph.event post-rewrite",
            ]
        );
    }

    #[test]
    fn a_hook_file_round_trips_to_its_exact_bytes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("post-commit");
        // Created, then removed: absent again.
        assert_eq!(install_file(&p, "L").unwrap(), FileOutcome::Created);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "#!/bin/sh\n# cartograph:start\nL\n# cartograph:end\n"
        );
        assert_eq!(
            install_file(&p, "L").unwrap(),
            FileOutcome::AlreadyThere,
            "idempotent"
        );
        assert_eq!(remove_file(&p).unwrap(), FileOutcome::Removed);
        assert!(!p.exists());
        // An existing hook, without a final newline: appended, then its exact bytes back.
        let before = "#!/usr/bin/env bash\necho mine";
        std::fs::write(&p, before).unwrap();
        assert_eq!(install_file(&p, "L").unwrap(), FileOutcome::Appended);
        assert_eq!(install_file(&p, "L2").unwrap(), FileOutcome::AlreadyThere);
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches(START).count(), 1, "one block, updated");
        assert!(text.contains("\nL2\n"));
        assert_eq!(remove_file(&p).unwrap(), FileOutcome::Restored);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
        // Not a shell script: refused, untouched.
        std::fs::write(&p, "#!/usr/bin/env python3\nprint()\n").unwrap();
        assert!(matches!(
            install_file(&p, "L").unwrap(),
            FileOutcome::Refused(_)
        ));
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "#!/usr/bin/env python3\nprint()\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_hook_is_never_followed() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("elsewhere");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        let p = d.path().join("post-merge");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        assert!(matches!(
            install_file(&p, "L").unwrap(),
            FileOutcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "#!/bin/sh\n");
    }
}
