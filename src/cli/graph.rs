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
    Mcp(GraphMcpArgs),
    /// Find unreferenced private symbols and dead code.
    DeadCode(GraphDeadCodeArgs),
    /// Find functions and methods ranked descending by cyclomatic complexity.
    Complexity(GraphComplexityArgs),
    /// Analyze class, struct, and trait inheritance/implementation hierarchy.
    Hierarchy(GraphHierarchyArgs),
    /// Generate a standalone zero-dependency HTML interactive graph visualizer.
    Viz(GraphVizArgs),
    /// Interactive browser-based codebase graph viewer.
    View(GraphViewArgs),
    /// Remove indexed repository data or clean the entire graph database.
    Clean(GraphCleanArgs),
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

#[derive(Debug, Args)]
pub struct GraphMcpArgs {
    /// Tools to advertise: `explore` lists only `graph_explore`, `all` lists every graph tool.
    #[arg(long, value_enum, default_value_t = crate::action::graph::mcp::ToolSurface::Explore)]
    pub tools: crate::action::graph::mcp::ToolSurface,
}

#[derive(Debug, Args)]
pub struct GraphDeadCodeArgs {
    /// Limit search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
    /// Maximum number of dead code items to return.
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GraphComplexityArgs {
    /// Minimum cyclomatic complexity threshold.
    #[arg(long, default_value_t = 10)]
    pub threshold: u32,
    /// Limit search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
    /// Maximum number of items to return.
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GraphHierarchyArgs {
    /// Symbol name to analyze inheritance/implementation hierarchy for.
    pub symbol: String,
    /// Limit search to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Debug, Args)]
pub struct GraphVizArgs {
    /// Output file path for the standalone HTML visualizer.
    #[arg(long, short = 'o', default_value = "graph.html")]
    pub output: String,
    /// Limit visualization to a specific repository.
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Debug, Args)]
pub struct GraphViewArgs {
    /// Start graph visualization focused on a specific repository.
    #[arg(long, group = "seed")]
    pub repo: Option<String>,
    /// Start graph visualization focused on a specific symbol.
    #[arg(long, group = "seed")]
    pub symbol: Option<String>,
    /// Start graph visualization focused on a specific file path.
    #[arg(long, group = "seed")]
    pub file: Option<String>,
    /// Start graph visualization focused on transitive impact of a symbol.
    #[arg(long, group = "seed")]
    pub impact: Option<String>,
    /// Maximum neighborhood depth hops (1..=2).
    #[arg(long)]
    pub depth: Option<usize>,
    /// Maximum number of nodes to load initially (1..=500).
    #[arg(long)]
    pub limit: Option<usize>,
    /// Start the server without opening the browser automatically.
    #[arg(long)]
    pub no_open: bool,
    /// Optional port to listen on (defaults to an ephemeral loopback port).
    #[arg(long)]
    pub port: Option<u16>,
}

impl From<GraphViewArgs> for crate::action::graph::input::GraphViewInput {
    fn from(args: GraphViewArgs) -> Self {
        let seed = if let Some(repo) = args.repo {
            crate::action::graph::view::types::ViewSeed::Repo(repo)
        } else if let Some(symbol) = args.symbol {
            crate::action::graph::view::types::ViewSeed::Symbol(symbol)
        } else if let Some(file) = args.file {
            crate::action::graph::view::types::ViewSeed::File(file)
        } else if let Some(impact) = args.impact {
            crate::action::graph::view::types::ViewSeed::Impact(impact)
        } else {
            crate::action::graph::view::types::ViewSeed::Default
        };

        Self {
            seed,
            depth: args.depth.unwrap_or(1),
            limit: args.limit.unwrap_or(400),
            no_open: args.no_open,
            port: args.port,
        }
    }
}

#[derive(Debug, Args)]
pub struct GraphCleanArgs {
    /// Specific repository to remove from the graph index.
    #[arg(long)]
    pub repo: Option<String>,
    /// Remove all repositories and data from the graph database.
    #[arg(long)]
    pub all: bool,
}

impl From<GraphCleanArgs> for crate::action::graph::input::CleanInput {
    fn from(args: GraphCleanArgs) -> Self {
        Self {
            repo: args.repo,
            all: args.all,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cli/graph.rs"]
mod tests;
