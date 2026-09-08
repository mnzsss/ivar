//! CLI argument definitions for `ivar graph` subcommands.

use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct GraphArgs {
    #[command(subcommand)]
    pub command: GraphCommand,
}

#[derive(Debug, Subcommand)]
pub enum GraphCommand {
    /// Hero query synthesizing symbol discovery, source snippet, callers, and impact.
    Explore(GraphExploreArgs),
    /// Find reverse-dependent test files for changed files.
    Affected(GraphAffectedArgs),
    /// Find shortest path between two symbols or files.
    Path(GraphPathArgs),
    /// Find symbols matching a query name pattern.
    Find(GraphFindArgs),
    /// List all callers of a symbol.
    Callers(GraphCallersArgs),
    /// List all outgoing calls (callees) from a symbol.
    Callees(GraphCalleesArgs),
    /// Show file outline with all defined symbols.
    File(GraphFileArgs),
    /// Incrementally update or build the codebase graph index.
    Index(GraphIndexArgs),
    /// Show overall graph statistics.
    Stats,
    /// Compute transitive blast-radius impact analysis for a symbol.
    Impact(GraphImpactArgs),
    /// Run graph MCP server.
    Mcp,
}

#[derive(Debug, Args)]
pub struct GraphExploreArgs {
    /// Symbol name or query pattern to explore.
    pub query: String,
    /// Limit exploration to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Debug, Args)]
pub struct GraphAffectedArgs {
    /// Changed file paths to find reverse dependencies for.
    pub files: Vec<String>,
    /// Read changed file paths from stdin (one per line).
    #[arg(long)]
    pub stdin: bool,
    /// Restrict search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
    /// Maximum search depth hops.
    #[arg(long)]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GraphPathArgs {
    /// Starting symbol name or file path.
    pub from: String,
    /// Target symbol name or file path.
    pub to: String,
    /// Maximum traversal hops.
    #[arg(long)]
    pub max_hops: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GraphFindArgs {
    /// Symbol name query pattern.
    pub query: String,
    /// Restrict search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
    /// Maximum number of matching symbols to return.
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GraphCallersArgs {
    /// Symbol name to find callers for.
    pub symbol: String,
    /// Restrict search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
    /// Search cross-repo callers.
    #[arg(long)]
    pub cross_repo: bool,
    /// Minimum edge confidence threshold (0.0 - 1.0).
    #[arg(long)]
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Args)]
pub struct GraphCalleesArgs {
    /// Symbol ID to find outgoing callees for.
    pub symbol_id: i64,
}

#[derive(Debug, Args)]
pub struct GraphFileArgs {
    /// Target repository name.
    pub repo: String,
    /// Target file path within the repository.
    pub path: String,
}

#[derive(Debug, Args)]
pub struct GraphIndexArgs {
    /// Specific repository to index (indexes all declared repos if omitted).
    #[arg(long)]
    pub repo: Option<String>,
    /// Force full reindex regardless of last indexed commit.
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Args)]
pub struct GraphImpactArgs {
    /// Symbol ID to compute blast radius impact for.
    pub symbol_id: i64,
    /// Maximum traversal depth.
    #[arg(long)]
    pub max_depth: Option<usize>,
}

#[cfg(test)]
#[path = "../../tests/unit/cli/graph.rs"]
mod tests;
