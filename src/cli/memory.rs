//! CLI argument definitions for `ivar memory` subcommands.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Show the hall memory database: path, schema version, application id and size.
    Status,
    /// Delete the hall memory database after confirmation. Nothing else deletes it.
    Clean,
}

#[cfg(test)]
#[path = "../../tests/unit/cli/memory.rs"]
mod tests;
