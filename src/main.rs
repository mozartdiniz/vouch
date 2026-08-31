//! `vouch` — a local, language-agnostic runtime for executing contract-checked functions.
//!
//! The property guaranteed is **value provenance**: every number a caller reports came from
//! a function's return value that satisfied its contracts, not from a model. Nodes may do
//! I/O freely; what is enforced is that the result passed its contracts.
//!
//! The runtime is sound but incomplete. It never returns an unsound answer. It may return
//! nothing.

mod attest;
mod commands;
mod contracts;
mod error;
mod exec;
mod ledger;
mod manifest;
mod registry;
mod schema;

use clap::{Parser, Subcommand};
use error::{Result, VouchError};
use registry::Registry;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "vouch",
    version,
    about = "Execute contract-checked functions, or refuse.",
    long_about = "Execute contract-checked functions, or refuse.\n\n\
                  Every call ends in one of two things: a value that satisfied its contracts, \
                  or a refusal with a machine-readable reason. There is no partial result.\n\n\
                  `vouch call` writes the result object to stdout; everything else goes to \
                  stderr."
)]
struct Cli {
    /// Run as if vouch had been started in <DIR> instead of the current directory
    #[arg(short = 'C', long = "directory", value_name = "DIR", global = true)]
    directory: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the nodes in this collection with their one-line purpose
    List,

    /// Show a node's full contract, parameters, guidance, and examples
    Describe {
        /// Node name, matching its directory under nodes/
        node: String,
        /// Emit the description as JSON
        #[arg(long)]
        json: bool,
    },

    /// Execute one verified call
    Call {
        /// Node name, matching its directory under nodes/
        node: String,
        /// Input object: '{json}', @file, or - for stdin
        #[arg(long, value_name = "INPUT")]
        input: String,
    },

    /// Check that every number in some prose came from the ledger
    ///
    /// No model is involved. Exits 0 if every numeral is accounted for, 1 if any is not,
    /// and 2 if the check itself could not be run.
    Attest {
        /// Ledger file (default: the most recent session in .vouch/ledger/)
        #[arg(long, value_name = "FILE")]
        ledger: Option<String>,
        /// Text to check: '...', @file, or - for stdin (default: stdin)
        #[arg(long, value_name = "TEXT", default_value = "-")]
        text: String,
        /// The user's original question; numbers quoted from it are not fabrication
        #[arg(long, value_name = "TEXT")]
        question: Option<String>,
        /// Also accept numbers that were passed *into* nodes, not just returned by them
        #[arg(long)]
        include_inputs: bool,
        /// Emit the report as JSON
        #[arg(long)]
        json: bool,
    },
}

/// `-C` changes the working directory before anything else happens, matching `git -C`:
/// everything downstream — collection discovery, a relative `--input @file`, a relative path
/// in an error message — behaves as though vouch had been started there.
fn enter(directory: &Path) -> Result<()> {
    std::env::set_current_dir(directory)
        .map_err(|e| VouchError::error(format!("cannot enter {}: {e}", directory.display())))
}

async fn run(cli: &Cli) -> Result<i32> {
    if let Some(directory) = &cli.directory {
        enter(directory)?;
    }
    let cwd = std::env::current_dir()
        .map_err(|e| VouchError::error(format!("cannot determine the working directory: {e}")))?;
    let registry = Registry::discover(&cwd)?;

    match &cli.command {
        Command::List => commands::list(&registry),
        Command::Describe { node, json } => commands::describe(&registry, node, *json),
        Command::Call { node, input } => commands::call(&registry, node, input).await,
        Command::Attest {
            ledger,
            text,
            question,
            include_inputs,
            json,
        } => commands::attest_text(
            &registry,
            ledger.as_deref(),
            text,
            question.as_deref(),
            *include_inputs,
            *json,
        ),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match run(&cli).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            e.emit();
            std::process::exit(e.code);
        }
    }
}
