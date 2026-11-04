//! CLI argument definitions for `ivar memory` subcommands.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Show the hall memory database: path, schema version, application id, size, observation counts per event, the capture spool, the last drain and the retention sweep.
    Status,
    /// Delete the hall memory database after confirmation. Nothing else deletes it.
    Clean,
    /// Record one session hook event into the capture spool. Always exits 0 and prints nothing.
    Capture {
        /// Provider whose hook payload arrives on stdin; without one nothing is recorded.
        #[arg(long)]
        provider: Option<String>,
    },
}

#[cfg(test)]
#[path = "../../tests/unit/cli/memory.rs"]
mod tests;
