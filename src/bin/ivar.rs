//! Entrypoint. Parses argv, dispatches into `action`, renders the outcome,
//! sets the exit code. No logic beyond that plumbing lives here — see
//! ARCHITECTURE.md's module map.

use std::process::ExitCode;

use clap::Parser;
use ivar::cli::root::Cli;

fn main() -> ExitCode {
    ivar::cli::run::run(Cli::parse())
}
