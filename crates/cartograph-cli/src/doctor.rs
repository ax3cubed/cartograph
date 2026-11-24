//! `cartograph doctor [--fix]` (SRV-24): checks the setup and prints one line per check, with the
//! exact command that fixes each problem.
//!
//! Each check implements [`Check`]; they run in parallel on plain threads. `--fix` runs only the
//! fixes that can be undone (`hooks install`, `schedule hourly`: each has its `remove` / `off`), then
//! runs that check again. No fix deletes the index or Ask history, and none edits a committed file:
//! the agents' files need `agents install`'s preview, so that one is printed, never run. The exit
//! code is 1 when any check fails.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cartograph_core::error::CoreError;
use cartograph_core::{ExitCode, Explain, Report, paths};
use cartograph_discovery::Registry;
use cartograph_discovery::registry::RepoEntry;
use serde_json::json;

use crate::output::Out;

/// What a check found. `fix_cmd` is printed; `auto_fix` is the `cartograph` arguments `--fix`
/// runs, only for a fix that can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass(String),
    Warn {
        why: String,
        fix_cmd: String,
        auto_fix: Option<Vec<String>>,
    },
    Fail {
        why: String,
        fix_cmd: String,
        auto_fix: Option<Vec<String>>,
    },
}

impl Outcome {
    fn word(&self) -> &'static str {
        match self {
            Outcome::Pass(_) => "pass",
            Outcome::Warn { .. } => "warn",
            Outcome::Fail { .. } => "fail",
        }
    }

    fn auto(&self) -> Option<&Vec<String>> {
        match self {
            Outcome::Warn { auto_fix, .. } | Outcome::Fail { auto_fix, .. } => auto_fix.as_ref(),
            Outcome::Pass(_) => None,
        }
    }
}

fn warn(why: impl Into<String>, fix_cmd: impl Into<String>) -> Outcome {
    Outcome::Warn {
        why: why.into(),
        fix_cmd: fix_cmd.into(),
        auto_fix: None,
    }
}

fn fail(why: impl Into<String>, fix_cmd: impl Into<String>) -> Outcome {
    Outcome::Fail {
        why: why.into(),
        fix_cmd: fix_cmd.into(),
        auto_fix: None,
    }
}

pub trait Check: Send + Sync {
    fn id(&self) -> String;
    fn run(&self) -> Outcome;
}

/// What every check may look at, read once.
struct Ctx {
    data: PathBuf,
    /// The data folder is inside a sync client's folder (B1).
    synced: Option<CoreError>,
    exe: PathBuf,
    home: Option<PathBuf>,
    ws: Option<Ws>,
}

struct Ws {
    id: String,
    /// The folder that holds `cartograph.yaml`.
    dir: PathBuf,
    manifest: PathBuf,
    repos: Vec<RepoEntry>,
}

fn shell_word(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

// ---------------------------------------------------------------- the checks

struct Git;
impl Check for Git {
    fn id(&self) -> String {
        "git".into()
    }
    fn run(&self) -> Outcome {
        match crate::hooks::git_version() {
            None => fail("git isn't on PATH", "install git 2.48 or later"),
            Some((a, b)) if (a, b) < (2, 48) => warn(
                format!(
                    "git {a}.{b} is older than 2.48, so `sync --fetch` can't fill in a missing origin/HEAD"
                ),
                "install git 2.48 or later",
            ),
            Some((a, b)) => Outcome::Pass(format!("git {a}.{b}")),
        }
    }
}

struct WorkspaceFile<'a>(&'a Ctx, &'a Path);
impl Check for WorkspaceFile<'_> {
    fn id(&self) -> String {
        "workspace".into()
    }
    fn run(&self) -> Outcome {
        let Some(ws) = &self.0.ws else {
            return warn(
                format!("no workspace covers {}", self.1.display()),
                format!("cartograph init {}", shell_word(self.1)),
            );
        };
        let file = match cartograph_discovery::manifest::read(&ws.manifest) {
            Ok(p) => p.manifest.workspace.file,
            Err(e) => return fail(e.why(), e.action()),
        };
        let Some(file) = file else {
            return Outcome::Pass(
                "no .code-workspace file; the folders were found by a walk".into(),
            );
        };
        let path = ws.dir.join(&file);
        match cartograph_discovery::resolve(&path) {
            Err(e) => fail(e.to_string(), e.action()),
            Ok(r) => {
                let literal: Vec<&String> = r
                    .warnings
                    .iter()
                    .filter(|w| w.contains("used literally"))
                    .collect();
                if literal.is_empty() {
                    Outcome::Pass(format!("{file} resolves"))
                } else {
                    warn(
                        literal
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                        format!(
                            "edit {}: write each of those paths relative to the file, without ~ or ${{env:…}}",
                            shell_word(&path)
                        ),
                    )
                }
            }
        }
    }
}

/// `.git/shallow` in the repo's common dir, as git itself decides.
fn shallow(repo: &Path) -> bool {
    let dot = repo.join(".git");
    let common = if dot.is_dir() {
        dot
    } else {
        let Some(gitdir) = std::fs::read_to_string(&dot)
            .ok()
            .and_then(|t| t.strip_prefix("gitdir:").map(|g| repo.join(g.trim())))
        else {
            return false;
        };
        match std::fs::read_to_string(gitdir.join("commondir")) {
            Ok(c) => gitdir.join(c.trim()),
            Err(_) => gitdir,
        }
    };
    common.join("shallow").is_file()
}

struct Repo<'a>(&'a Ctx, &'a RepoEntry);
impl Check for Repo<'_> {
    fn id(&self) -> String {
        format!(
            "repo {}",
            self.1
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    }
    fn run(&self) -> Outcome {
        let p = &self.1.path;
        let dir = self.0.ws.as_ref().map_or(Path::new("."), |w| &w.dir);
        if !p.join(".git").exists() {
            return fail(
                format!(
                    "{} isn't a git repo any more; init reads the workspace again and registers the repos it finds",
                    p.display()
                ),
                format!("cartograph init {}", shell_word(dir)),
            );
        }
        if shallow(p) {
            return warn(
                "a shallow clone: history before its first commit isn't indexed",
                format!("git -C {} fetch --unshallow", shell_word(p)),
            );
        }
        Outcome::Pass(p.display().to_string())
    }
}

struct Index<'a>(&'a Ctx);
impl Check for Index<'_> {
    fn id(&self) -> String {
        "index".into()
    }
    fn run(&self) -> Outcome {
        let c = self.0;
        if let Some(e) = &c.synced {
            return fail(
                format!("{}: {}", e.what(), e.why()),
                format!(
                    "move {} out of the synced folder and point CARTOGRAPH_HOME at its new place",
                    shell_word(&c.data)
                ),
            );
        }
        let Some(ws) = &c.ws else {
            return Outcome::Pass("no workspace here".into());
        };
        let path = paths::workspace_dir(&c.data, &ws.id).join("index.cartograph");
        let sync = format!("cartograph sync {}", shell_word(&ws.dir));
        if !path.exists() {
            return fail("there is no index yet", sync);
        }
        match cartograph_store::Index::open_read(&path) {
            Err(e) => fail(format!("{}: {}", e.what(), e.why()), e.action()),
            Ok(ix) => match ix.state {
                cartograph_store::State::Current => Outcome::Pass(format!(
                    "format {} at {}",
                    cartograph_store::FORMAT_VERSION,
                    path.display()
                )),
                cartograph_store::State::NeedsMigrate => {
                    warn("the index was written by an older Cartograph", sync)
                }
                cartograph_store::State::NewerReadable => warn(
                    "the index was written by a newer Cartograph; this one reads it only",
                    "install the newer cartograph that wrote it",
                ),
            },
        }
    }
}

/// The binary a hook line runs: the quoted first word.
fn line_binary(line: &str) -> Option<String> {
    let rest = line.strip_prefix('\'')?;
    let end = rest.find("' sync")?;
    Some(rest[..end].replace("'\\''", "'"))
}

struct Hooks<'a>(&'a Ctx);
impl Check for Hooks<'_> {
    fn id(&self) -> String {
        "hooks".into()
    }
    fn run(&self) -> Outcome {
        let Some(ws) = &self.0.ws else {
            return Outcome::Pass("no workspace here".into());
        };
        let want = crate::hooks::hook_line(&self.0.exe);
        let lines: Vec<Option<String>> = ws
            .repos
            .iter()
            .map(|r| {
                crate::hooks::installed_line(&r.path, crate::hooks::hooks_dir(&r.path).as_deref())
            })
            .collect();
        if lines.iter().all(Option::is_none) {
            return Outcome::Pass("not installed (optional: `cartograph hooks install`)".into());
        }
        let stale: Vec<&String> = lines.iter().flatten().filter(|l| **l != want).collect();
        let fix = vec![
            "hooks".to_owned(),
            "install".into(),
            ws.dir.to_string_lossy().into_owned(),
        ];
        let fix_cmd = format!("cartograph hooks install {}", shell_word(&ws.dir));
        if let Some(l) = stale.first() {
            let bin = line_binary(l).unwrap_or_default();
            let why = format!(
                "the hooks run {bin}, not this binary ({})",
                self.0.exe.display()
            );
            return if Path::new(&bin).is_file() {
                Outcome::Warn {
                    why,
                    fix_cmd,
                    auto_fix: Some(fix),
                }
            } else {
                Outcome::Fail {
                    why: format!("{why}, and it is gone"),
                    fix_cmd,
                    auto_fix: Some(fix),
                }
            };
        }
        let missing = lines.iter().filter(|l| l.is_none()).count();
        if missing > 0 {
            return Outcome::Warn {
                why: format!("{missing} of {} repos have no hooks", lines.len()),
                fix_cmd,
                auto_fix: Some(fix),
            };
        }
        Outcome::Pass(format!("installed in {} repos", lines.len()))
    }
}

struct Schedule<'a>(&'a Ctx);
impl Check for Schedule<'_> {
    fn id(&self) -> String {
        "schedule".into()
    }
    fn run(&self) -> Outcome {
        let Some((job, stored)) = crate::schedule::installed() else {
            return Outcome::Pass("no hourly sync (optional: `cartograph schedule hourly`)".into());
        };
        if Path::new(&stored) == self.0.exe {
            return Outcome::Pass(format!("hourly, {job}"));
        }
        let why = format!(
            "{job} runs {stored}, not this binary ({})",
            self.0.exe.display()
        );
        let fix = || Some(vec!["schedule".to_owned(), "hourly".into()]);
        if Path::new(&stored).is_file() {
            Outcome::Warn {
                why,
                fix_cmd: "cartograph schedule hourly".into(),
                auto_fix: fix(),
            }
        } else {
            Outcome::Fail {
                why: format!("{why}, and it is gone"),
                fix_cmd: "cartograph schedule hourly".into(),
                auto_fix: fix(),
            }
        }
    }
}

struct Agents<'a>(&'a Ctx);
impl Check for Agents<'_> {
    fn id(&self) -> String {
        "agents".into()
    }
    fn run(&self) -> Outcome {
        let Some(home) = &self.0.home else {
            return warn(
                "HOME isn't set, so the agents' config can't be found",
                "set HOME",
            );
        };
        let rows =
            cartograph_server::agents::status(home, self.0.ws.as_ref().map(|w| w.dir.as_path()));
        let missing: Vec<&str> = rows
            .iter()
            .filter(|r| r.detected && !r.installed)
            .map(|r| r.name.as_str())
            .collect();
        if !missing.is_empty() {
            return warn(
                format!("{} found but not set up", missing.join(", ")),
                "cartograph agents install",
            );
        }
        let set_up = rows.iter().filter(|r| r.installed).count();
        // The entries start `cartograph` by name, so PATH must find a cartograph binary.
        let on_path = std::env::var_os("PATH").is_some_and(|p| {
            std::env::split_paths(&p).any(|d| {
                d.join(if cfg!(windows) {
                    "cartograph.exe"
                } else {
                    "cartograph"
                })
                .is_file()
            })
        });
        if set_up > 0 && !on_path {
            let dir = self.0.exe.parent().unwrap_or(Path::new("."));
            return warn(
                "the agents start `cartograph` by name, and PATH doesn't find it",
                format!("add {} to PATH", shell_word(dir)),
            );
        }
        Outcome::Pass(format!("{set_up} set up"))
    }
}

struct Clones<'a>(&'a Ctx);
impl Check for Clones<'_> {
    fn id(&self) -> String {
        "clones".into()
    }
    fn run(&self) -> Outcome {
        let reg = match Registry::load(&self.0.data) {
            Ok((r, _)) => r,
            Err(e) => return fail(e.why(), e.action()),
        };
        let dups = reg.duplicate_clones();
        if dups.is_empty() {
            return Outcome::Pass("each repo is registered once".into());
        }
        let dir = self.0.ws.as_ref().map_or(Path::new("."), |w| &w.dir);
        warn(
            dups.join("; "),
            format!(
                "keep one of each in the workspace, then run `cartograph init {}`",
                shell_word(dir)
            ),
        )
    }
}

// ---------------------------------------------------------------- running them

fn context(dir: &Path) -> Result<Ctx, Report> {
    let (data, synced) = match paths::data_dir() {
        Ok(d) => (d, None),
        Err(CoreError::SyncedDataDir { path }) => {
            let e = CoreError::SyncedDataDir { path: path.clone() };
            (PathBuf::from(path), Some(e))
        }
        Err(e) => return Err(Report::from_error(&e)),
    };
    let ws = cartograph_discovery::setup::locate(&data, dir)
        .ok()
        .flatten()
        .and_then(|a| {
            let (reg, _) = Registry::load(&data).ok()?;
            let w = reg.workspaces.iter().find(|w| w.id == a.workspace_id)?;
            Some(Ws {
                id: w.id.clone(),
                dir: w.manifest.parent()?.to_owned(),
                manifest: w.manifest.clone(),
                repos: reg
                    .repos
                    .iter()
                    .filter(|r| r.workspace == w.id)
                    .cloned()
                    .collect(),
            })
        });
    Ok(Ctx {
        data,
        synced,
        exe: std::env::current_exe()
            .map_err(|e| crate::io_report("can't find the cartograph binary".into(), e))?,
        home: crate::agents::home().ok(),
        ws,
    })
}

fn line(id: &str, o: &Outcome) -> String {
    match o {
        Outcome::Pass(note) => format!("ok   {id}: {note}"),
        Outcome::Warn { why, fix_cmd, .. } => format!("warn {id}: {why} · fix: {fix_cmd}"),
        Outcome::Fail { why, fix_cmd, .. } => format!("FAIL {id}: {why} · fix: {fix_cmd}"),
    }
}

pub fn run(out: &Out, dir: &Path, fix: bool) -> Result<(), Report> {
    let ctx = context(dir)?;
    let mut checks: Vec<Box<dyn Check + '_>> = vec![
        Box::new(Git),
        Box::new(WorkspaceFile(&ctx, dir)),
        Box::new(Index(&ctx)),
    ];
    if let Some(ws) = &ctx.ws {
        checks.extend(
            ws.repos
                .iter()
                .map(|r| Box::new(Repo(&ctx, r)) as Box<dyn Check>),
        );
    }
    checks.push(Box::new(Hooks(&ctx)));
    checks.push(Box::new(Schedule(&ctx)));
    checks.push(Box::new(Agents(&ctx)));
    checks.push(Box::new(Clones(&ctx)));

    let mut results: Vec<(String, Outcome, bool)> = std::thread::scope(|s| {
        let handles: Vec<_> = checks.iter().map(|c| s.spawn(|| c.run())).collect();
        checks
            .iter()
            .zip(handles)
            .map(|(c, h)| {
                let o = h
                    .join()
                    .unwrap_or_else(|_| fail("the check panicked", "cartograph report"));
                (c.id(), o, false)
            })
            .collect()
    });
    if fix {
        for (i, (_, o, fixed)) in results.iter_mut().enumerate() {
            let Some(args) = o.auto().cloned() else {
                continue;
            };
            let ran = Command::new(&ctx.exe)
                .args(&args)
                .arg("--quiet")
                .stdin(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ran {
                *o = checks[i].run();
                *fixed = true;
            }
        }
    }
    let text: Vec<String> = results
        .iter()
        .map(|(id, o, fixed)| {
            let l = line(id, o);
            if *fixed { format!("{l} (fixed)") } else { l }
        })
        .collect();
    let rows: Vec<_> = results
        .iter()
        .map(|(id, o, fixed)| {
            let (why, fix_cmd) = match o {
                Outcome::Pass(n) => (n.clone(), None),
                Outcome::Warn { why, fix_cmd, .. } | Outcome::Fail { why, fix_cmd, .. } => {
                    (why.clone(), Some(fix_cmd.clone()))
                }
            };
            json!({ "id": id, "status": o.word(), "why": why, "fix": fix_cmd,
                    "auto_fix": o.auto().is_some(), "fixed": fixed })
        })
        .collect();
    out.print(&text.join("\n"), json!({ "checks": rows }));
    let failed: Vec<&str> = results
        .iter()
        .filter(|(_, o, _)| matches!(o, Outcome::Fail { .. }))
        .map(|(id, _, _)| id.as_str())
        .collect();
    if failed.is_empty() {
        return Ok(());
    }
    Err(Report {
        what: format!("{} checks failed: {}", failed.len(), failed.join(", ")),
        why: "each line above says what is wrong".into(),
        action: "run the fix printed under each failed check".into(),
        exit_code: ExitCode::Error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_line_names_its_binary() {
        let l = crate::hooks::hook_line(Path::new("/opt/it's here/cartograph"));
        assert_eq!(
            line_binary(&l).as_deref(),
            Some("/opt/it's here/cartograph")
        );
    }

    #[test]
    fn one_line_per_check_with_the_fix() {
        assert_eq!(
            line("git", &Outcome::Pass("git 2.50".into())),
            "ok   git: git 2.50"
        );
        assert_eq!(
            line(
                "index",
                &fail("there is no index yet", "cartograph sync /w")
            ),
            "FAIL index: there is no index yet · fix: cartograph sync /w"
        );
        assert_eq!(shell_word(Path::new("/a b/c")), "'/a b/c'");
    }
}
