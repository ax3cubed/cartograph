//! `cartograph mcp`, `mcp --list`, `tool` and `impact` (SRV-15, SRV-17, SRV-11): each calls the
//! one tool registry in `cartograph_server::mcp`, so the shell sees exactly what an agent sees.

use std::io::Read;
use std::path::Path;

use cartograph_core::{ExitCode, Report, logging};
use cartograph_server::mcp::{Tools, registry, registry_summary, stdio};
use serde_json::{Value, json};

use crate::output::Out;
use crate::{Format, data_dir};

fn tools(dir: &Path) -> Result<Tools, Report> {
    Ok(Tools::new(data_dir()?, dir.to_owned()))
}

/// `cartograph mcp`: the server on stdin and stdout. Logs go to `logs/` (and stderr with -v);
/// stdout carries protocol frames only.
pub fn serve(dir: &Path, verbose: u8) -> Result<(), Report> {
    let data = data_dir()?;
    let _guard = logging::init(&data.join("logs"), verbose).ok();
    stdio::serve(Tools::new(data, dir.to_owned())).map_err(|e| Report {
        what: "the MCP server stopped".into(),
        why: e.to_string(),
        action: "check the log in the data folder's logs/".into(),
        exit_code: ExitCode::Error,
    })
}

/// `cartograph mcp --list`: the registry.
pub fn list(out: &Out) {
    let human = registry()
        .iter()
        .map(|t| format!("{:<10} {}", t.wire_name(), t.description))
        .collect::<Vec<_>>()
        .join("\n");
    out.print(&human, json!({"schema": 1, "tools": registry_summary()}));
}

/// `cartograph tool <name> [json]`: the tool's `structuredContent` as JSON, or its Markdown with
/// `--format md`. The arguments come from the argument or, without one, stdin.
pub fn tool(name: &str, args: Option<String>, format: Format, dir: &Path) -> Result<(), Report> {
    let text = match args {
        Some(a) => a,
        None => {
            let mut s = String::new();
            std::io::stdin()
                .take(1 << 20)
                .read_to_string(&mut s)
                .map_err(|e| usage("can't read the arguments from stdin", &e.to_string()))?;
            s
        }
    };
    let args: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|e| usage("the arguments aren't JSON", &e.to_string()))?
    };
    if !registry().iter().any(|t| t.wire_name() == name) {
        return Err(usage(
            &format!("there is no tool named {name}"),
            "the tools are status, find, describe, neighbors, path, impact, sync and correct",
        ));
    }
    print(tools(dir)?.call(name, args, None), format)
}

fn usage(what: &str, why: &str) -> Report {
    Report {
        what: what.into(),
        why: why.into(),
        action: "for example: cartograph tool find '{\"query\":\"invoice\"}'".into(),
        exit_code: ExitCode::Usage,
    }
}

fn print(o: cartograph_server::mcp::Output, format: Format) -> Result<(), Report> {
    if o.is_error {
        return Err(Report {
            what: o.content.clone(),
            why: "the tool returned an error".into(),
            action: "the message says what to try".into(),
            exit_code: ExitCode::Error,
        });
    }
    match format {
        Format::Md => println!("{}", o.content),
        Format::Json => println!(
            "{}",
            serde_json::to_string(&o.structured.unwrap_or(Value::Null)).unwrap_or_default()
        ),
    }
    Ok(())
}

/// `cartograph impact <ref>`: a commit range on the indexed branch (`A..B`, `A..`) is the files
/// changed after A. A branch or the working tree is refused by name until branch overlays land
/// (IDX-24, phase 5).
pub fn impact(
    reference: Option<&str>,
    working_tree: bool,
    format: Format,
    dir: &Path,
) -> Result<(), Report> {
    let args = match (reference, working_tree) {
        (_, true) => json!({"working_tree": true}),
        (Some(r), false) => match r.split_once("..") {
            Some((since, "")) | Some((since, "HEAD")) if !since.is_empty() => {
                json!({"since": since, "limit": 200})
            }
            _ => json!({"ref": r}),
        },
        (None, false) => json!({}),
    };
    print(tools(dir)?.call("impact", args, None), format)
}
