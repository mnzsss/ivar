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
    /// Open, list or cancel a handoff for the next session. Accepting one and messages are MCP-only.
    Handoff {
        #[command(subcommand)]
        command: HandoffCommand,
    },
    /// Record one session hook event into the capture spool. Always exits 0 and prints nothing.
    /// When a session with no feature ends, it also starts `ivar hall sync` in a process of its
    /// own, logging to `.ivar/hall-sync.log`, and does not wait for it; `auto_commit: false`
    /// in `ivar.json` turns that off.
    Capture {
        /// Provider whose hook payload arrives on stdin; without one nothing is recorded.
        #[arg(long)]
        provider: Option<String>,
    },
    /// Print the open handoffs and pending messages this session can see, for a session-start hook, and one line when the hall's sync is blocked. Always exits 0; prints nothing when there is nothing to show.
    Brief {
        /// Provider whose hook runs this; used when the session is outside `ivar session`.
        #[arg(long)]
        provider: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum HandoffCommand {
    /// Open a handoff in this session's feature queue, or the hall's outside a feature.
    Begin {
        /// Where the work stopped, in markdown.
        #[arg(long)]
        summary: String,
        /// A task the next session should pick up (repeatable).
        #[arg(long = "pending")]
        pending: Vec<String>,
        /// A decision still open (repeatable).
        #[arg(long = "decision")]
        decision: Vec<String>,
        /// A file the work touched (repeatable).
        #[arg(long = "file")]
        file: Vec<String>,
        /// Open it in this feature's queue instead.
        #[arg(long)]
        feature: Option<String>,
        /// Open it in the hall queue.
        #[arg(long, conflicts_with = "feature")]
        hall: bool,
        /// Only sessions of this provider see it.
        #[arg(long)]
        to_provider: Option<String>,
        /// Only this ivar session sees it.
        #[arg(long, conflicts_with = "to_provider")]
        to_session: Option<String>,
        /// Open it on behalf of this ivar session, which becomes its owner.
        #[arg(long)]
        session: Option<String>,
    },
    /// List open handoffs. Read-only.
    List {
        /// Only this feature's queue.
        #[arg(long)]
        feature: Option<String>,
        /// Only the hall queue.
        #[arg(long, conflicts_with = "feature")]
        hall: bool,
    },
    /// Cancel an open handoff, whoever owns it.
    Cancel {
        /// The handoff id, as `list` shows it.
        id: String,
    },
}

#[cfg(test)]
#[path = "../../tests/unit/cli/memory.rs"]
mod tests;
