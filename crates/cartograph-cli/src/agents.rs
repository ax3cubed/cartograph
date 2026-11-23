//! `cartograph agents install | remove | status` (SRV-21, F5, F6): the CLI face of
//! `cartograph_server::agents`, which the app's `/api/v1/agents` routes call too.
//!
//! `install` sets up every agent found on this machine (its config folder or its binary on PATH);
//! `remove` takes Cartograph out of every agent's files. Each change is shown as a diff and
//! confirmed first (E3), unless `--yes`.

use std::path::{Path, PathBuf};

use cartograph_core::{ExitCode, Report};
use cartograph_discovery::Registry;
use cartograph_server::agents::{self, Agent, Place, Scope};
use serde_json::json;

use crate::output::{Out, sanitize};

/// The user's home folder, where the agents keep their config.
pub fn home() -> Result<PathBuf, Report> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Report {
            what: "can't find your home folder".into(),
            why: "neither HOME nor USERPROFILE is set".into(),
            action: "set HOME and run it again".into(),
            exit_code: ExitCode::Error,
        })
}

/// The folder that holds the workspace's `cartograph.yaml`, and its registered repos, when `dir`
/// is in a registered workspace.
pub fn workspace_of(dir: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let data = crate::data_dir().ok()?;
    let applied = cartograph_discovery::setup::locate(&data, dir).ok()??;
    let (reg, _) = Registry::load(&data).ok()?;
    let ws = reg
        .workspaces
        .iter()
        .find(|w| w.id == applied.workspace_id)?;
    let repos = reg
        .repos
        .iter()
        .filter(|r| r.workspace == ws.id)
        .map(|r| r.path.clone())
        .collect();
    Some((ws.manifest.parent()?.to_owned(), repos))
}

fn report(e: &agents::AgentError) -> Report {
    Report::from_error(e)
}

pub fn run(
    out: &Out,
    dir: &Path,
    action: crate::Setup,
    scope: Scope,
    yes: bool,
) -> Result<(), Report> {
    let home = home()?;
    let (workspace, repos) = workspace_of(dir).unzip();
    if let crate::Setup::Status = action {
        let rows = agents::status(&home, workspace.as_deref());
        let plugin = agents::plugin_installed(&home);
        let mut text: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}: {}, {}{}",
                    r.name,
                    if r.detected { "found" } else { "not found" },
                    if r.installed { "set up" } else { "not set up" },
                    r.config_path
                        .as_deref()
                        .map(|p| format!(" ({p})"))
                        .unwrap_or_default()
                )
            })
            .collect();
        if plugin {
            text.push("the Claude Code plugin is installed".into());
        }
        out.print(
            &text.join("\n"),
            json!({ "agents": rows, "claude_code_plugin": plugin }),
        );
        return Ok(());
    }
    let install = matches!(action, crate::Setup::Install);
    let which: Vec<Agent> = Agent::ALL
        .into_iter()
        .filter(|a| !install || agents::detected(&home, *a))
        .collect();
    if which.is_empty() {
        out.print(
            "no coding agents found on this machine (Claude Code, Cursor, VS Code, Codex)",
            json!({ "changes": [] }),
        );
        return Ok(());
    }
    let place = Place {
        home,
        workspace,
        repos: repos.unwrap_or_default(),
    };
    let changes = agents::plan(&place, scope, &which, install).map_err(|e| report(&e))?;
    if changes.is_empty() {
        out.print("nothing to change", json!({ "changes": [] }));
        return Ok(());
    }
    let preview: String = changes.iter().map(|c| c.diff()).collect();
    if !yes {
        for line in preview.lines() {
            eprintln!("{}", sanitize(line));
        }
    }
    out.confirm(
        &format!(
            "{} {} files?",
            if install { "Write" } else { "Change" },
            changes.len()
        ),
        yes,
    )?;
    agents::apply(&changes).map_err(|e| report(&e))?;
    let mut text = if yes { preview } else { String::new() };
    for c in &changes {
        let verb = match (&c.old, &c.new) {
            (None, _) => "created",
            (_, None) => "deleted",
            _ => "updated",
        };
        text += &format!("{verb} {} ({})\n", c.path.display(), c.agent.name());
    }
    out.print(
        &text,
        json!({ "changes": changes.iter().map(|c| json!({
            "agent": c.agent.id(),
            "file": c.path.to_string_lossy(),
            "diff": c.diff(),
        })).collect::<Vec<_>>() }),
    );
    Ok(())
}
