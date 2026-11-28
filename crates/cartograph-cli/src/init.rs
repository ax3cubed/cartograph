//! `cartograph init`: find the workspace, show what will be indexed and skipped, write
//! cartograph.yaml, record the workspace in the registry, then build the first index.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use cartograph_core::{Report, logging, paths};
use cartograph_discovery::manifest::{self, Repo};
use cartograph_discovery::setup::{self, Plan, PlannedRepo};
use cartograph_facts::FactCache;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use serde_json::json;

use crate::output::{Out, sanitize};
use crate::{Global, data_dir, lossy};

fn report(e: &dyn cartograph_core::Explain) -> Report {
    Report::from_error(e)
}

/// Each repo found on this machine with its entry in the merged manifest. The manifest keeps the
/// committed order and the entries not cloned here, while `plan.repos` follows the resolution
/// (sorted by root), so the two lists aren't parallel: pair them as `plan` matched them, by folder,
/// else by name for a committed entry whose folder isn't on this machine.
fn entries(plan: &Plan) -> Vec<(&Repo, &PlannedRepo)> {
    let base = plan.manifest_path.parent().unwrap_or(Path::new("."));
    let at: Vec<Option<PathBuf>> = plan
        .manifest
        .repos
        .iter()
        .map(|e| std::fs::canonicalize(manifest::repo_dir(base, e)).ok())
        .collect();
    plan.repos
        .iter()
        .zip(&plan.resolution.repos)
        .filter_map(|(p, r)| {
            let i = at
                .iter()
                .position(|a| a.as_deref() == Some(p.root.as_path()))
                .or_else(|| {
                    let n = setup::repo_name(&r.name);
                    (0..at.len()).find(|&i| at[i].is_none() && plan.manifest.repos[i].name == n)
                })?;
            Some((&plan.manifest.repos[i], p))
        })
        .collect()
}

/// The plan as the spec's first-run listing.
fn listing(plan: &Plan) -> String {
    let mut s = String::new();
    if let Some(f) = &plan.workspace_file {
        let n = plan
            .resolution
            .repos
            .iter()
            .map(|r| r.scopes.len().max(1))
            .sum::<usize>()
            + plan.resolution.skipped.len();
        s.push_str(&format!("Found {} with {n} folders.\n", f.display()));
    }
    for (r, p) in entries(plan) {
        let name = match &r.display {
            Some(d) => format!("{d} ({})", r.name),
            None => r.name.clone(),
        };
        let mut line = format!("  ✓ {name:<24} {}", p.branch);
        if !r.scopes.is_empty() {
            line.push_str(&format!(
                "   {} scopes: {}",
                r.scopes.len(),
                r.scopes.join(", ")
            ));
        }
        s.push_str(&line);
        s.push('\n');
    }
    for k in &plan.resolution.skipped {
        s.push_str(&format!(
            "  – {:<24} skipped: {}\n",
            k.folder,
            k.reason.as_str()
        ));
    }
    for w in &plan.warnings {
        s.push_str(&format!("  ! {w}\n"));
    }
    s
}

pub fn run(out: &Out, dir: &Path, no_index: bool, g: &Global) -> Result<(), Report> {
    // Discovery walks absolute paths; `init r` and `init .` mean the same folders as their full paths.
    let dir = &dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let plan = setup::plan(dir, g.workspace.as_deref()).map_err(|e| report(&e))?;
    let text = listing(&plan);
    if plan.repos.is_empty() {
        return Err(Report {
            what: "nothing to index".into(),
            why: format!("no folder of the workspace is in a git work tree\n{text}"),
            action: "run `git init` in a folder, or add a repo to the workspace file".into(),
            exit_code: cartograph_core::ExitCode::Usage,
        });
    }
    // Ask before writing anything. Without a terminal and without --yes, write the manifest only.
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let index_now = !no_index && (g.yes || terminal);
    if index_now && !g.yes {
        for line in text.lines() {
            eprintln!("{}", sanitize(line));
        }
        out.confirm(&format!("Index {} repos?", plan.repos.len()), false)?;
    }

    let data = data_dir()?;
    let applied = setup::apply(&plan, &data).map_err(|e| report(&e))?;
    // The first index is the longest write this command does: its warnings (a resumed run, a
    // checkpoint that fell back) go to the workspace's log as `sync`'s do, and to stderr with -v.
    let _log = logging::init(&applied.workspace_dir.join("logs"), g.verbose).ok();
    let mut done = vec![format!("✓ wrote {}", plan.manifest_path.display())];
    let mut index = None;
    if index_now {
        let ws = Workspace {
            dir: applied.workspace_dir.clone(),
            repos: entries(&plan)
                .into_iter()
                .map(|(r, p)| RepoSpec {
                    name: r.name.clone(),
                    path: p.root.clone(),
                    branch: Some(p.branch.clone()),
                    scopes: r.scopes.clone(),
                })
                .collect(),
            exclude: plan.manifest.options.exclude.clone(),
            config_hash: Some(cartograph_discovery::manifest::config_hash(&plan.manifest)),
            services: plan
                .manifest
                .services
                .iter()
                .map(|s| cartograph_index::structure::ServiceSpec {
                    name: s.name.clone(),
                    repo: s.repo.clone(),
                    path: s.path.clone(),
                })
                .collect(),
            contracts: plan
                .manifest
                .contracts
                .iter()
                .map(cartograph_index::contracts::ContractSpec::from)
                .collect(),
            clients: plan
                .manifest
                .clients
                .iter()
                .map(cartograph_index::entrypoints::ClientSpec::from)
                .collect(),
            environments: plan
                .manifest
                .environments
                .iter()
                .map(|(name, e)| cartograph_index::config::EnvSpec {
                    name: name.clone(),
                    sources: e.sources.clone(),
                    set: e.set.clone(),
                })
                .collect(),
            default_environment: plan.manifest.default_environment.clone(),
            overrides: plan
                .manifest_path
                .parent()
                .map(|d| d.join(cartograph_discovery::overrides::FILE_NAME)),
            subs: Default::default(),
        };
        let cache_path = paths::facts_cache(&data);
        let mut cache = FactCache::open(&cache_path).map_err(|e| Report {
            what: format!("can't open the fact cache at {}", cache_path.display()),
            why: e.to_string(),
            action: format!("delete {}; it is only a cache", cache_path.display()),
            exit_code: cartograph_core::ExitCode::Error,
        })?;
        match sync(&ws, &SyncOptions::default(), &mut cache).map_err(|e| report(&e))? {
            SyncOutcome::Synced(s) => done.push(format!(
                "✓ indexed {} commits in {} ms\n  {}",
                s.steps,
                s.total_ms,
                applied.index.display()
            )),
            SyncOutcome::AlreadyRunning => {
                done.push("a sync is already running; it will include this workspace".into())
            }
        }
        index = Some(lossy(&applied.index));
    } else {
        done.push("Next: `cartograph sync` builds the index.".into());
    }
    // A fresh init leaves no group- or world-readable file in the data folder (phase 2 exit gate).
    paths::make_private(&data).map_err(|e| Report {
        what: format!("can't make {} user-only", data.display()),
        why: e.to_string(),
        action: format!("run `chmod -R go-rwx {}`", data.display()),
        exit_code: cartograph_core::ExitCode::Error,
    })?;

    let human = if index_now && !g.yes {
        done.join("\n")
    } else {
        format!("{text}{}", done.join("\n"))
    };
    out.print(
        &human,
        json!({
            "workspace": applied.workspace_id,
            "manifest": lossy(&plan.manifest_path),
            "index": index,
            "repos": plan.manifest.repos,
            "skipped": plan.resolution.skipped,
            "warnings": plan.warnings,
        }),
    );
    Ok(())
}
