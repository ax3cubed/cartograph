//! The `cartograph` binary: argument parsing, logging, the error contract and the exit codes.
//!
//! Start-up stays cheap: clap parses first, `--help` and `--version` exit inside clap, and each
//! command resolves the data folder, starts logging or opens an index only once it runs. There is
//! no async runtime here; `serve`, `open` and `mcp --http` bring their own when they land.

mod adapters;
mod agents;
mod app;
mod check;
mod doctor;
mod hooks;
mod init;
mod mcp;
mod output;
mod schedule;
mod status;
mod sync;
mod verify;

use std::path::{Path, PathBuf};
use std::process::ExitCode as ProcessExit;

use cartograph_core::{ExitCode, Report, logging, paths};
use cartograph_facts::FactCache;
use cartograph_facts::cache::create_private_dir;
use clap::{Args, FromArgMatches, Parser, Subcommand, ValueEnum};
// Linked now so the binary's dependency graph is fixed; used later in phase 1.
use cartograph_query as _;
use serde_json::json;

use crate::output::Out;

#[derive(Parser)]
#[command(
    name = "cartograph",
    version,
    about = "A codebase map with history.",
    after_help = "Every command takes [dir], the current folder by default. Exit codes: 0 done, \
                  1 failed, 2 bad arguments or no workspace, 3 `status --check` found a repo behind."
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Global {
    /// The .code-workspace file to use instead of finding one.
    #[arg(long, global = true, value_name = "FILE")]
    workspace: Option<PathBuf>,
    /// Print one JSON object, {"schema":1,…}, on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Print nothing but errors.
    #[arg(short, long, global = true)]
    quiet: bool,
    /// No colour, as NO_COLOR does.
    #[arg(long, global = true)]
    no_color: bool,
    /// Answer yes to every question; needed for destructive commands without a terminal.
    #[arg(short, long, global = true)]
    yes: bool,
    /// More output on stderr: -v info, -vv debug, -vvv trace.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

/// The folder a command works on, as the last positional argument (clig.dev). A command run
/// inside any repo of a workspace finds the workspace. `--dir` is the older spelling, kept hidden
/// so scripts keep working; it is also the way to name a folder after an optional positional of
/// the command's own (`open`, `ask`, `impact`, `verify`, `tool`).
#[derive(Args)]
struct Dir {
    /// The folder to work on [default: the current folder].
    dir: Option<PathBuf>,
    #[arg(long = "dir", value_name = "DIR", hide = true, conflicts_with = "dir")]
    dir_flag: Option<PathBuf>,
}

impl Dir {
    /// The folder named, or the current one.
    fn or_cwd(self) -> PathBuf {
        self.dir
            .or(self.dir_flag)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Setup {
    Install,
    Remove,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum Schedule {
    Hourly,
    Off,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Md,
    Json,
}

// ponytail: the flags of commands that print "not built yet" are parsed and never read. Each
// command's plan item reads them; drop this allow when the last one lands.
#[allow(dead_code)]
#[derive(Subcommand)]
enum Command {
    // First index and keeping it current.
    /// Find the workspace, write cartograph.yaml, build the first index.
    Init {
        #[command(flatten)]
        dir: Dir,
        /// Write the manifest only; don't index.
        #[arg(long)]
        no_index: bool,
        /// Also install the git hooks.
        #[arg(long)]
        hooks: bool,
        /// Also set up the coding agents found on this machine.
        #[arg(long)]
        agents: bool,
    },
    /// Index the commits added since the last run.
    Sync {
        #[command(flatten)]
        dir: Dir,
        /// Fetch each repo's remote first.
        // The background child is a plain `sync --quiet`: refused together rather than dropped.
        #[arg(long, conflicts_with = "background")]
        fetch: bool,
        /// Wait for a sync that is already running.
        #[arg(long, conflicts_with = "background")]
        wait: bool,
        /// Run in the background and return at once.
        #[arg(long)]
        background: bool,
        /// Every registered workspace, one after another (what the hourly schedule runs).
        #[arg(long, conflicts_with_all = ["background", "dir"])]
        all_workspaces: bool,
        /// Run an approved subprocess adapter with no sandbox, on a system that has none.
        #[arg(long, conflicts_with = "background")]
        unsandboxed_adapters: bool,
        /// The git hook's arguments (IDX-26): a file checkout or a rebase in progress starts
        /// nothing.
        #[arg(long, hide = true, requires = "background", value_name = "ARGS")]
        hook_args: Option<String>,
    },
    /// List, allow or deny the subprocess adapters cartograph.yaml names; `guide` explains them.
    // Hidden: help lists the seventeen commands of the spec's section 4.
    #[command(hide = true)]
    Adapters {
        action: adapters::Action,
        /// The adapter's name, for allow and deny.
        name: Option<String>,
        /// The folder to work on [default: the current folder].
        #[arg(long = "dir", value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Each repo's indexed commit, its HEAD and how far behind it is.
    Status {
        #[command(flatten)]
        dir: Dir,
        /// Exit 3 when any repo is behind.
        #[arg(long)]
        check: bool,
        /// One line with counts and fixed words only, for a session-start hook.
        #[arg(long)]
        line: bool,
        /// List every import that didn't resolve, with its file and line.
        #[arg(long, conflicts_with = "line")]
        unresolved: bool,
    },
    /// Index one repository's history (temporary single-repo form, IDX-07).
    Index {
        #[command(flatten)]
        dir: Dir,
        /// Build the index again from nothing, then rename it into place.
        #[arg(long)]
        rebuild: bool,
        /// Where to write the index [default: <data>/workspaces/<id>/index.cartograph].
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Compare fact hashes with another index or the manifest.
    Verify {
        /// The other index.
        other: Option<PathBuf>,
        /// Compare with the committed manifest instead.
        #[arg(long, conflicts_with = "other")]
        manifest: bool,
        #[command(flatten)]
        dir: Dir,
    },

    // Optional ways to stay current.
    /// Install, remove or show the git hooks that sync after each commit.
    Hooks {
        action: Setup,
        #[command(flatten)]
        dir: Dir,
    },
    /// Sync every workspace hourly, or stop doing so.
    Schedule { action: Schedule },

    // Reading the map.
    /// Serve the app on 127.0.0.1 and open it in the browser.
    Open {
        /// A cartograph:// link to open at.
        link: Option<String>,
        #[command(flatten)]
        dir: Dir,
    },
    /// Run the MCP server over stdio.
    Mcp {
        #[command(flatten)]
        dir: Dir,
        /// Listen on loopback HTTP instead, with Host, Origin and token checks.
        #[arg(long)]
        http: bool,
        /// List the tools and exit.
        #[arg(long)]
        list: bool,
    },
    /// Run one MCP tool and print its JSON result.
    Tool {
        /// The tool's name.
        name: String,
        /// The tool's arguments, as JSON [default: read from stdin].
        args: Option<String>,
        /// Print the structured JSON, or the Markdown an agent reads.
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
        #[command(flatten)]
        dir: Dir,
    },
    /// What a branch, a commit range or your uncommitted changes would affect.
    Impact {
        /// A branch or a commit range.
        #[arg(name = "ref", required_unless_present = "working_tree")]
        reference: Option<String>,
        /// Use the uncommitted changes.
        #[arg(long)]
        working_tree: bool,
        /// Wait for a sync to finish first.
        #[arg(long)]
        wait: bool,
        /// Output format.
        #[arg(long, value_enum, default_value = "md")]
        format: Format,
        #[command(flatten)]
        dir: Dir,
    },
    /// Fail on stale corrections or broken architecture rules.
    Check {
        #[command(flatten)]
        dir: Dir,
        /// Write cartograph.baseline.json: accept the rule violations there are now.
        #[arg(long)]
        update_baseline: bool,
    },
    /// Ask the map a question.
    Ask {
        /// The question.
        #[arg(required_unless_present_any = ["history", "forget"])]
        question: Option<String>,
        /// Answer as of this date or commit.
        #[arg(long)]
        at: Option<String>,
        /// Show past questions.
        #[arg(long)]
        history: bool,
        /// Delete your Ask history.
        #[arg(long)]
        forget: bool,
        #[command(flatten)]
        dir: Dir,
    },

    // Agents, health and cleanup.
    /// Write MCP config for the coding agents on this machine.
    Agents {
        action: Setup,
        /// In the project instead of at user level.
        #[arg(long)]
        project: bool,
        /// In each repo.
        #[arg(long)]
        per_repo: bool,
        /// In a file the team commits.
        #[arg(long)]
        shared: bool,
        #[command(flatten)]
        dir: Dir,
    },
    /// Check the setup and print the command that fixes each problem.
    Doctor {
        #[command(flatten)]
        dir: Dir,
        /// Run the fixes that can be undone.
        #[arg(long)]
        fix: bool,
    },
    /// Write a report of the last failure, with paths and code removed. Nothing is sent.
    Report {
        #[command(flatten)]
        dir: Dir,
    },
    /// Delete the cache, or with --all the index too.
    Clean {
        #[command(flatten)]
        dir: Dir,
        /// The parse cache.
        #[arg(long)]
        cache: bool,
        /// The cache and the index.
        #[arg(long)]
        all: bool,
    },

    /// Print where Cartograph keeps its data (B1).
    #[command(hide = true)]
    Paths,
    /// Serve the app without opening a browser; prints the address (SRV-05).
    #[command(hide = true)]
    Serve {
        /// A loopback address or localhost [default: 127.0.0.1].
        #[arg(long)]
        host: Option<String>,
        /// The port [default: the last one, when free].
        #[arg(long)]
        port: Option<u16>,
        /// Serve this data folder instead, for front-end work (debug builds, HD-04). Make one
        /// with `cargo run -p cartograph-fixtures --example fixture-home -- <dir>`.
        #[cfg(debug_assertions)]
        #[arg(long, value_name = "DIR")]
        fixture: Option<PathBuf>,
    },
}

fn main() -> ProcessExit {
    // Clap prints usage errors and exits 2 (E2), or prints --help and --version and exits 0,
    // before anything else runs.
    let matches = <Cli as clap::CommandFactory>::command().get_matches();
    let name = matches.subcommand_name().unwrap_or_default().to_owned();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let out = Out::new(cli.global.json, cli.global.quiet, cli.global.no_color);
    let code = match run(cli, &out, &name) {
        Ok(()) => ExitCode::Ok,
        Err(report) => {
            out.error(&report);
            report.exit_code
        }
    };
    ProcessExit::from(code as u8)
}

fn run(cli: Cli, out: &Out, name: &str) -> Result<(), Report> {
    let g = cli.global;
    match cli.command {
        Command::Paths => {
            let data = data_dir()?;
            out.print(
                &format!(
                    "data       {}\nfacts      {}\nworkspaces {}",
                    data.display(),
                    paths::facts_cache(&data).display(),
                    data.join("workspaces").display()
                ),
                json!({
                    "data": lossy(&data),
                    "facts": lossy(&paths::facts_cache(&data)),
                    "workspaces": lossy(&data.join("workspaces")),
                }),
            );
        }
        Command::Status {
            dir,
            check,
            line,
            unresolved,
        } => {
            status::run(out, &named(&g, dir.or_cwd()), check, line, unresolved)?;
        }
        Command::Index {
            dir,
            rebuild,
            out: file,
        } => {
            index(out, &dir.or_cwd(), file, rebuild, &g)?;
        }
        Command::Init {
            dir,
            no_index,
            hooks: false,
            agents: false,
        } => init::run(out, &dir.or_cwd(), no_index, &g)?,
        Command::Verify {
            other,
            manifest: false,
            dir,
        } => verify::run(out, &named(&g, dir.or_cwd()), other.as_deref())?,
        Command::Open { .. } => app::open(out, &g)?,
        #[cfg(debug_assertions)]
        Command::Serve {
            host,
            port,
            fixture,
        } => app::serve(out, &g, host, port, fixture)?,
        #[cfg(not(debug_assertions))]
        Command::Serve { host, port } => app::serve(out, &g, host, port, None)?,
        Command::Sync {
            dir,
            fetch,
            wait,
            background,
            all_workspaces,
            unsandboxed_adapters,
            hook_args,
        } => sync::run(
            out,
            &named(&g, dir.or_cwd()),
            sync::Flags {
                fetch,
                wait,
                background,
                all_workspaces,
                unsandboxed_adapters,
                hook_args,
            },
            &g,
        )?,
        Command::Adapters { action, name, dir } => adapters::run(
            out,
            &named(&g, dir.unwrap_or_else(|| PathBuf::from("."))),
            action,
            name,
        )?,
        Command::Hooks { action, dir } => hooks::run(out, &named(&g, dir.or_cwd()), action)?,
        Command::Schedule { action } => schedule::run(out, action)?,
        Command::Mcp {
            list: true,
            http: false,
            ..
        } => mcp::list(out),
        Command::Mcp {
            dir,
            http: false,
            list: false,
        } => mcp::serve(&dir.or_cwd(), g.verbose)?,
        Command::Tool {
            name,
            args,
            format,
            dir,
        } => mcp::tool(&name, args, format, &dir.or_cwd())?,
        Command::Impact {
            reference,
            working_tree,
            wait: _,
            format,
            dir,
        } => mcp::impact(reference.as_deref(), working_tree, format, &dir.or_cwd())?,
        Command::Agents {
            action,
            project,
            per_repo,
            shared,
            dir,
        } => agents::run(
            out,
            &named(&g, dir.or_cwd()),
            action,
            cartograph_server::agents::Scope {
                project,
                per_repo,
                shared,
            },
            g.yes,
        )?,
        Command::Check {
            dir,
            update_baseline,
        } => check::run(out, &named(&g, dir.or_cwd()), update_baseline)?,
        Command::Doctor { dir, fix } => doctor::run(out, &named(&g, dir.or_cwd()), fix)?,
        _ => return Err(not_built(name)),
    }
    Ok(())
}

/// The folder whose workspace `status`, `sync` and `verify` act on. `--workspace FILE` names it by
/// its .code-workspace file, beside which init wrote cartograph.yaml (C5), and wins over [dir].
fn named(g: &Global, dir: PathBuf) -> PathBuf {
    match g
        .workspace
        .as_deref()
        .map(|f| f.parent().unwrap_or(Path::new("")))
    {
        Some(p) if p.as_os_str().is_empty() => PathBuf::from("."),
        Some(p) => p.to_owned(),
        None => dir,
    }
}

fn not_built(name: &str) -> Report {
    Report {
        what: format!("`cartograph {name}` is not built yet"),
        why: "this early build registers every command, but only some of them work".into(),
        action: "run `cartograph index <dir>` to index one repository".into(),
        exit_code: ExitCode::Error,
    }
}

/// A path for JSON. serde_json would refuse a path that isn't UTF-8.
fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn data_dir() -> Result<PathBuf, Report> {
    paths::data_dir().map_err(|e| Report::from_error(&e))
}

/// Where the temporary single-repo index of `repo` lives (until the registry, IDX-11).
fn default_index(data: &Path, repo: &Path) -> PathBuf {
    paths::workspace_dir(data, &cartograph_index::workspace_id(repo)).join("index.cartograph")
}

fn io_report(what: String, e: std::io::Error) -> Report {
    Report {
        what,
        why: e.to_string(),
        action: "check the folder's permissions and the free disk space".into(),
        exit_code: ExitCode::Error,
    }
}

fn index(
    out: &Out,
    repo: &Path,
    file: Option<PathBuf>,
    rebuild: bool,
    g: &Global,
) -> Result<(), Report> {
    let data = data_dir()?;
    let _guard = logging::init(&data.join("logs"), g.verbose).ok();
    let target = file.unwrap_or_else(|| default_index(&data, repo));
    if let Some(dir) = target.parent().filter(|d| !d.as_os_str().is_empty()) {
        create_private_dir(dir)
            .map_err(|e| io_report(format!("can't create {}", dir.display()), e))?;
    }
    // A rebuild builds beside the old file under the writer lock and renames it into place, so the
    // old index stays readable until the new one is complete (B2, IDX-16).
    let rebuild = rebuild && target.exists();
    if rebuild {
        // --rebuild replaces an index, never another program's file: one that isn't an index is
        // refused here, before the lock file and the rename touch anything beside it.
        if let Err(cartograph_store::StoreError::NotAnIndex { .. }) =
            cartograph_store::Index::open_read(&target)
        {
            return Err(Report {
                what: format!("{} isn't a Cartograph index", target.display()),
                why: "--rebuild replaces only an index; this file belongs to something else".into(),
                action: "choose another --out path, or move that file away first".into(),
                exit_code: ExitCode::Error,
            });
        }
        out.confirm(
            &format!("Rebuild {} from nothing?", target.display()),
            g.yes,
        )?;
    }
    let cache_path = paths::facts_cache(&data);
    let mut cache = FactCache::open(&cache_path).map_err(|e| Report {
        what: format!("can't open the fact cache at {}", cache_path.display()),
        why: e.to_string(),
        action: format!("delete {}; it is only a cache", cache_path.display()),
        exit_code: ExitCode::Error,
    })?;
    let workers = cartograph_index::default_workers();
    let s = if rebuild {
        let dir = target.parent().unwrap_or(Path::new("."));
        let lock = cartograph_store::writer::WriterLock::try_acquire(dir)
            .map_err(|e| io_report(format!("can't lock {}", dir.display()), e))?
            .ok_or_else(|| Report {
                what: format!("{} is being written by another process", target.display()),
                why: "only one writer at a time (IDX-13)".into(),
                action: "wait for it to finish, then run the rebuild again".into(),
                exit_code: ExitCode::Error,
            })?;
        cartograph_index::rebuild_single(&lock, repo, None, &target, &mut cache, workers)
    } else {
        cartograph_index::build_single(repo, None, &target, &mut cache, workers)
    }
    .map_err(|e| Report::from_error(&e))?;
    out.print(
        &format!(
            "indexed {} commits into {}\n\
             {} links, {} intervals ({} open at head), {} resolutions\n\
             {} blobs: {} parsed, {} from the cache, {} skipped\n\
             {} ms (walk {}, extract {}, replay and write {})",
            s.steps,
            target.display(),
            s.links,
            s.intervals,
            s.open_at_head,
            s.resolutions,
            s.extract.blobs,
            s.extract.parsed,
            s.extract.from_cache,
            s.extract.skipped,
            s.total_ms,
            s.walk_ms,
            s.extract_ms,
            s.replay_ms
        ),
        json!({
            "index": lossy(&target),
            "commits": s.steps,
            "links": s.links,
            "intervals": s.intervals,
            "open_at_head": s.open_at_head,
            "resolutions": s.resolutions,
            "blobs": { "total": s.extract.blobs, "parsed": s.extract.parsed,
                       "from_cache": s.extract.from_cache, "skipped": s.extract.skipped },
            "ms": { "total": s.total_ms, "walk": s.walk_ms, "extract": s.extract_ms,
                    "replay": s.replay_ms },
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn help_lists_the_seventeen_commands_of_spec_section_4() {
        let visible: Vec<_> = super::Cli::command()
            .get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| c.get_name().to_owned())
            .collect();
        assert_eq!(
            visible,
            [
                "init", "sync", "status", "index", "verify", "hooks", "schedule", "open", "mcp",
                "tool", "impact", "check", "ask", "agents", "doctor", "report", "clean"
            ]
        );
    }

    #[test]
    fn the_cli_definition_is_consistent() {
        super::Cli::command().debug_assert();
    }
}
