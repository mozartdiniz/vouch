//! `vouch` — a local, language-agnostic runtime for executing contract-checked functions.
//!
//! The property guaranteed is **value provenance**: every number a caller reports came from
//! a function's return value that satisfied its contracts, not from a model. Nodes may do
//! I/O freely; what is enforced is that the result passed its contracts.
//!
//! The runtime is sound but incomplete. It never returns an unsound answer. It may return
//! nothing.

mod attest;
mod cases;
mod commands;
mod contracts;
mod error;
mod eval;
mod exec;
mod ledger;
mod manifest;
mod markdown;
mod registry;
mod schema;
mod verify;

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
    ///
    /// With --all, describes the whole collection as a markdown routing pack: paste it into
    /// a CLAUDE.md, or have an agent run it at session start.
    Describe {
        /// Node name, matching its directory under nodes/. Omit it with --all.
        node: Option<String>,
        /// Describe every node in the collection, with its registry preamble
        #[arg(long)]
        all: bool,
        /// Emit markdown (the default for --all)
        #[arg(long, conflicts_with = "json")]
        md: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Emit JSON sized for a context window: routing fields only, no whitespace
        #[arg(long, conflicts_with = "md")]
        compact: bool,
        /// Emit only names, purposes and when to reach for them, for picking a shortlist
        #[arg(long, conflicts_with_all = ["md", "compact"])]
        index: bool,
    },

    /// Execute one verified call
    Call {
        /// Node name, matching its directory under nodes/
        node: String,
        /// Input object: '{json}', @file, or - for stdin
        #[arg(long, value_name = "INPUT")]
        input: String,
    },

    /// Run node fixtures from cases.toml
    ///
    /// Deterministic and fast: fixed input, expected exit code, expected values. Nothing is
    /// written to the ledger. Exits 0 if every case passed, 1 if any failed, and 2 if the run
    /// itself could not happen.
    Test {
        /// Node name, matching its directory under nodes/. Omit it to test every node.
        node: Option<String>,
        /// Emit the report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run agent routing evals against a natural-language suite
    ///
    /// A model is in the loop, so this reports a rate, not a pass: each case runs -n times.
    /// Costs tokens on every run. Exits 0 if the pass rate meets --min-rate, 1 if it does
    /// not, and 2 if the run itself could not happen.
    Eval {
        /// Eval suite (default: .vouch/evals.toml)
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
        /// Command that takes a prompt and prints a reply; {prompt} is substituted if present
        #[arg(long, value_name = "CMD", default_value = "claude -p")]
        agent: String,
        /// How many times to run each case
        #[arg(short = 'n', value_name = "N", default_value_t = 1)]
        runs: usize,
        /// Pass rate the suite must reach, from 0.0 to 1.0
        #[arg(long, value_name = "RATE", default_value_t = 1.0)]
        min_rate: f64,
        /// Skip runs a previous invocation already finished
        #[arg(long)]
        resume: bool,
        /// Stop after this many agent calls. Calls, not dollars: the agent is any command and
        /// reports a reply, not a bill
        #[arg(long, value_name = "N")]
        max_calls: Option<usize>,
        /// Emit the report as JSON
        #[arg(long)]
        json: bool,
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
        Command::Describe {
            node,
            all,
            md,
            json,
            compact,
            index,
        } => {
            let format = match (md, json, compact, index) {
                // Both imply JSON: they are shapes of it, and asking for `--compact --json`
                // to get compact JSON would be a trap for no gain.
                (_, _, _, true) => commands::Format::Index,
                (_, _, true, _) => commands::Format::Compact,
                (_, true, _, _) => commands::Format::Json,
                (true, _, _, _) => commands::Format::Markdown,
                // A whole collection defaults to the pack; a single node to prose.
                _ if *all => commands::Format::Markdown,
                _ => commands::Format::Human,
            };
            match (node, all) {
                (Some(_), true) => Err(VouchError::error(
                    "describe takes either a node name or --all, not both",
                )),
                (Some(name), false) => commands::describe(&registry, name, format),
                (None, true) => commands::describe_all(&registry, format),
                (None, false) => Err(VouchError::error(
                    "describe needs a node name, or --all for the whole collection",
                )),
            }
        }
        Command::Call { node, input } => commands::call(&registry, node, input).await,
        Command::Test { node, json } => commands::test(&registry, node.as_deref(), *json).await,
        Command::Eval {
            file,
            agent,
            runs,
            min_rate,
            json,
            resume,
            max_calls,
        } => {
            commands::eval(
                &registry,
                file.as_deref(),
                agent,
                *runs,
                *min_rate,
                *json,
                *resume,
                *max_calls,
            )
            .await
        }
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
