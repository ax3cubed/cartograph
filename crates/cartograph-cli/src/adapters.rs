//! `cartograph adapters list | allow <name> | deny <name> | guide` (XS-24): the only way a subprocess
//! adapter named in a committed manifest gets approved. The approval is the adapter folder's hash
//! and its resolved command path, kept in the per-user registry (C4), so a changed adapter, or the
//! same manifest on another machine, asks again. A sync, MCP and the background refresh never write
//! it.

use std::path::Path;

use cartograph_core::{ExitCode, Report};
use cartograph_discovery::{Registry, manifest, setup};
use cartograph_facts::adapters::subprocess::{Declared, Sandbox};
use serde_json::json;

use crate::data_dir;
use crate::output::{Out, sanitize};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Action {
    List,
    Allow,
    Deny,
    /// How to write an adapter, and where to ask for one.
    Guide,
}

/// The static guide, in the binary: no network.
const GUIDE: &str = "\
Subprocess adapters read files Cartograph has no built-in adapter for.

A folder inside the repository holds adapter.toml:
    name = \"flask\"          # lowercase letters, digits and dashes
    version = \"1\"           # change it when the output changes: facts are cached per blob and version
    command = \"python3\"     # a program on a standard path, or ./file inside the folder
    args = [\"adapter.py\"]
    files = [\"**/*.py\"]     # globs over repository paths

Name the folder in cartograph.yaml under `adapters:`, then on each machine read it and run
`cartograph adapters allow flask`. Any change to the folder asks again.

The protocol is NDJSON on stdin and stdout. One process runs per sync. For each file Cartograph writes
    {\"type\":\"extract\",\"id\":1,\"blob\":\"<id>\",\"path\":\"app/a.py\",\"content\":\"<the file>\"}
and the adapter answers with any number of fact lines, then a done line:
    {\"type\":\"fact\",\"id\":1,\"blob\":\"<id>\",\"path\":\"app/a.py\",\"kind\":\"route\",
     \"span\":[1,0,1,9],\"attrs\":{\"method\":\"GET\",\"path\":\"/x\"},
     \"evidence\":{\"type\":\"literal\",\"text\":\"@get('/x')\"}}
    {\"type\":\"done\",\"id\":1}
id, blob and path must echo the request; a line that names another file is rejected and the adapter
stopped. Spans are 1-based lines and 0-based columns inside the file. Kinds: route, http.call,
event.publish, event.subscribe, db.read, db.write, db.table.def, env.read, client.base, entrypoint,
symbol.def, symbol.ref.

The adapter runs with no network, no file writes and no reads outside its folder and the system's
runtime, an empty environment, 10 s per file, 4 MiB of output and 10000 facts per file. Where there is
no sandbox (Windows) it is refused unless you pass `cartograph sync --unsandboxed-adapters`.
";

const REQUEST_URL: &str =
    "https://github.com/ax3cubed/cartograph/issues/new?title=Request+an+adapter";

fn fail(what: impl Into<String>, why: impl Into<String>, action: impl Into<String>) -> Report {
    Report {
        what: what.into(),
        why: why.into(),
        action: action.into(),
        exit_code: ExitCode::Error,
    }
}

pub fn run(out: &Out, dir: &Path, action: Action, name: Option<String>) -> Result<(), Report> {
    if matches!(action, Action::Guide) {
        out.print(
            &format!("{GUIDE}\nRequest an adapter: {REQUEST_URL}"),
            json!({ "request_url": REQUEST_URL }),
        );
        return Ok(());
    }
    let data = data_dir()?;
    let applied = setup::locate(&data, dir)
        .map_err(|e| Report::from_error(&e))?
        .ok_or_else(|| Report {
            what: format!("no workspace covers {}", dir.display()),
            why: "no `cartograph init` registered a repo containing this folder".into(),
            action: "run `cartograph init` in the workspace folder".into(),
            exit_code: ExitCode::Usage,
        })?;
    let id = applied.workspace_id;
    let (reg, _) = Registry::load(&data).map_err(|e| Report::from_error(&e))?;
    let entry = reg.workspaces.iter().find(|w| w.id == id).ok_or_else(|| {
        fail(
            "the workspace is not registered",
            "",
            "run `cartograph init`",
        )
    })?;
    let parsed = manifest::read(&entry.manifest).map_err(|e| Report::from_error(&e))?;
    let base = entry.manifest.parent().unwrap_or(Path::new("/"));
    let declared: Vec<(String, Result<Declared, String>)> = parsed
        .manifest
        .adapters
        .iter()
        .map(|f| {
            (
                f.clone(),
                Declared::load(base, f).map_err(|e| e.to_string()),
            )
        })
        .collect();
    match action {
        Action::List => {
            let sandbox = Sandbox::detect();
            let mut lines = Vec::new();
            let mut rows = Vec::new();
            for (folder, d) in &declared {
                let (state, detail) = match d {
                    Ok(d) if reg.adapter_allowed(&id, &d.name, &d.hash) => {
                        ("allowed", d.name.clone())
                    }
                    Ok(d)
                        if reg
                            .adapters
                            .iter()
                            .any(|g| g.workspace == id && g.name == d.name) =>
                    {
                        (
                            "changed",
                            format!("{}: changed since you allowed it", d.name),
                        )
                    }
                    Ok(d) => ("not approved", d.name.clone()),
                    Err(e) => ("invalid", e.clone()),
                };
                lines.push(format!("{state}: {} ({folder})", sanitize(&detail)));
                rows.push(json!({ "folder": folder, "state": state, "detail": detail }));
            }
            if lines.is_empty() {
                lines.push("no subprocess adapters in cartograph.yaml".into());
            }
            lines.push(format!("sandbox: {sandbox:?}"));
            out.print(
                &lines.join("\n"),
                json!({ "adapters": rows, "sandbox": format!("{sandbox:?}") }),
            );
        }
        Action::Allow | Action::Deny => {
            let name = name.ok_or_else(|| Report {
                what: "no adapter name".into(),
                why: "allow and deny take the adapter's name".into(),
                action: "run `cartograph adapters list`, then `cartograph adapters allow <name>`"
                    .into(),
                exit_code: ExitCode::Usage,
            })?;
            let found = declared
                .iter()
                .filter_map(|(_, d)| d.as_ref().ok())
                .find(|d| d.name == name);
            if matches!(action, Action::Deny) {
                let removed = Registry::update(&data, |r| r.deny_adapter(&id, &name))
                    .map_err(|e| Report::from_error(&e))?;
                let word = if removed {
                    "removed"
                } else {
                    "was not allowed"
                };
                out.print(
                    &format!("{name} {word}"),
                    json!({ "name": name, "removed": removed }),
                );
                return Ok(());
            }
            let d = found.ok_or_else(|| {
                fail(
                    format!("no valid adapter named {name} in cartograph.yaml"),
                    "it is not listed under `adapters:`, or its folder does not load",
                    "run `cartograph adapters list`",
                )
            })?;
            Registry::update(&data, |r| r.allow_adapter(&id, &d.name, &d.hash))
                .map_err(|e| Report::from_error(&e))?;
            out.print(
                &format!(
                    "allowed {} {}: it runs {} {} from {}, in a sandbox with no network and no writes",
                    d.name,
                    d.version,
                    d.command.display(),
                    d.args.join(" "),
                    d.dir.display()
                ),
                json!({ "name": d.name, "hash": d.hash, "command": d.command }),
            );
        }
        Action::Guide => unreachable!("handled above"),
    }
    Ok(())
}
