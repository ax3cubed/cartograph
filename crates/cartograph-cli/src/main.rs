//! The `cartograph` binary. Phase 0 skeleton: argument parsing, logging, the error contract and the
//! exit codes. Commands arrive with their items (init, sync, open, ... in phase 1).

use std::process::ExitCode as ProcessExit;

use cartograph_core::{ExitCode, Report, logging, paths};
use clap::{Parser, Subcommand};
// Linked now so the binary's dependency graph is fixed; used from phase 1.
use cartograph_index as _;
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

fn run(cli: Cli) -> Result<(), Report> {
    let data = paths::data_dir().map_err(|e| Report::from_error(&e))?;
    let _guard = logging::init(&data.join("logs"), cli.verbose).ok();
    match cli.command {
        Command::Paths => {
            println!("data       {}", data.display());
            println!("facts      {}", paths::facts_cache(&data).display());
            println!("workspaces {}", data.join("workspaces").display());
        }
    }
    Ok(())
}
