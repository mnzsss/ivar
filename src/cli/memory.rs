//! CLI argument definitions for `ivar memory` subcommands.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Show the hall memory database: path, schema version, application id, size, observation counts per event, the capture spool, the last drain and the retention sweep.
    Status,
    /// Delete the hall memory database after confirmation. Nothing else deletes it.
    Clean,
    /// Search the pages under docs/memory. When no page matches and no filter is set, retained observations answer instead.
    Query {
        /// Words to search for.
        text: String,
        /// Keep only pages of this kind (repeatable).
        #[arg(long)]
        kind: Vec<String>,
        /// Keep only pages of this tier: semantic, procedural, episodic or working.
        #[arg(long)]
        tier: Option<String>,
        /// Keep only pages carrying this exact tag.
        #[arg(long)]
        tag: Option<String>,
        /// Most results to return (at most 20).
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Also return pages whose expires_at has passed.
        #[arg(long)]
        include_expired: bool,
        /// Show how each source ranked every result.
        #[arg(long)]
        explain: bool,
    },
    /// Rebuild the page index from docs/memory. Observations are not touched.
    Reindex,
    /// Check the pages under docs/memory against the page rules. Exits non-zero when any rule is broken.
    Lint,
    /// Show what a feature wrote to memory, its gaps and the pages its diff may have made stale, plus the last review. Read-only: confirm with the memory_review MCP tool.
    Review {
        /// Feature to review. Defaults to the session's feature.
        feature: Option<String>,
    },
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
