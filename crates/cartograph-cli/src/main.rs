//! The `cartograph` binary: argument parsing, logging, the error contract and the exit codes.

use std::path::PathBuf;
use std::process::ExitCode as ProcessExit;

use cartograph_core::{ExitCode, Report, logging, paths};
use cartograph_facts::FactCache;
use cartograph_facts::cache::create_private_dir;
use clap::{Parser, Subcommand};
// Linked now so the binary's dependency graph is fixed; used later in phase 1.
use cartograph_query as _;

#[derive(Parser)]
#[command(name = "cartograph", version, about = "A codebase map with history.")]
struct Cli {
    /// More output on stderr: -v info, -vv debug, -vvv trace.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print where Cartograph keeps its data (B1).
    Paths,
    /// Index one repository's first-parent history into a new index file (temporary, IDX-07).
    Index {
        /// The repository folder.
        repo: PathBuf,
        /// Where to write the index. Default: <data>/workspaces/<id>/index.cartograph. Never
        /// overwritten.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ProcessExit {
    // clap exits with 2 on bad usage, which is E2's code for it.
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ProcessExit::from(i32::from(ExitCode::Ok) as u8),
        Err(report) => {
            eprintln!("{report}");
            ProcessExit::from(i32::from(report.exit_code) as u8)
        }
    }
}

fn io_report(what: String, e: std::io::Error) -> Report {
    Report {
        what,
        why: e.to_string(),
        action: "check the folder's permissions and the free disk space".into(),
        exit_code: ExitCode::Error,
    }
}

fn run(cli: Cli) -> Result<(), Report> {
    let data = paths::data_dir().map_err(|e| Report::from_error(&e))?;
    let _guard = logging::init(&data.join("logs"), cli.verbose).ok();
    match cli.command {
        Command::Paths => {
            println!("data       {}", data.display());
            println!("facts      {}", paths::facts_cache(&data).display());
            println!("workspaces {}", data.join("workspaces").display());
        }
        Command::Index { repo, out } => {
            let out = out.unwrap_or_else(|| {
                paths::workspace_dir(&data, &cartograph_index::workspace_id(&repo))
                    .join("index.cartograph")
            });
            if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
                create_private_dir(dir)
                    .map_err(|e| io_report(format!("can't create {}", dir.display()), e))?;
            }
            let cache_path = paths::facts_cache(&data);
            let mut cache = FactCache::open(&cache_path).map_err(|e| Report {
                what: format!("can't open the fact cache at {}", cache_path.display()),
                why: e.to_string(),
                action: format!("delete {}; it is only a cache", cache_path.display()),
                exit_code: ExitCode::Error,
            })?;
            let s = cartograph_index::build_single(
                &repo,
                None,
                &out,
                &mut cache,
                cartograph_index::default_workers(),
            )
            .map_err(|e| Report::from_error(&e))?;
            println!("indexed {} commits into {}", s.steps, out.display());
            println!(
                "{} links, {} intervals ({} open at head), {} resolutions",
                s.links, s.intervals, s.open_at_head, s.resolutions
            );
            println!(
                "{} blobs: {} parsed, {} from the cache, {} skipped",
                s.extract.blobs, s.extract.parsed, s.extract.from_cache, s.extract.skipped
            );
            println!(
                "{} ms (walk {}, extract {}, replay and write {})",
                s.total_ms, s.walk_ms, s.extract_ms, s.replay_ms
            );
        }
    }
    Ok(())
}
