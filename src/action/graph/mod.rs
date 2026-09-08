pub mod affected;
pub mod cross_repo;
pub mod explore;
pub mod index;
pub mod mcp;
pub mod path;
pub mod query;

pub use affected::{AffectedError, find_affected_tests, is_test_file, parse_files_from_reader};
pub use cross_repo::{CrossRepoLinkOutcome, link_cross_repo_edges};
pub use explore::{ExploreError, explore};
pub use index::{IndexOutcome, index_repo};
pub use mcp::run_mcp_server;
pub use path::{PathError, find_shortest_path};
pub use query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactItem, ImpactResult, QueryError, SymbolLocation,
    find_symbols, get_callees, get_callers, get_file_outline, get_graph_stats, get_impact,
};
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::action::Ctx;
use crate::action::discover_hall;
use crate::action::read_manifest;

#[derive(Debug, Clone)]
pub struct ExploreInput {
    pub query: String,
    pub repo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AffectedInput {
    pub files: Vec<String>,
    pub stdin: bool,
    pub repo: Option<String>,
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PathInput {
    pub from: String,
    pub to: String,
    pub max_hops: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FindInput {
    pub query: String,
    pub repo: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CallersInput {
    pub symbol: String,
    pub repo: Option<String>,
    pub cross_repo: bool,
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CalleesInput {
    pub symbol_id: i64,
}

#[derive(Debug, Clone)]
pub struct FileInput {
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct IndexInput {
    pub repo: Option<String>,
    pub full: bool,
}

#[derive(Debug, Clone)]
pub struct ImpactInput {
    pub symbol_id: i64,
    pub max_depth: Option<usize>,
}
use crate::domain::graph::{AffectedResult, ExploreResult, GraphStats, PathResult};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::graph::db::GraphDb;

fn open_graph_db(ctx: &Ctx) -> Result<GraphDb, Failure> {
    let layout = discover_hall(ctx)?;
    let db_path = layout.ivar_dir().join("memory.db");
    GraphDb::open(db_path.as_std_path()).map_err(|err| {
        Failure::failed(
            "graph.db_open_failed",
            format!("Failed to open graph database at {db_path}: {err}"),
        )
    })
}

// 1. explore
pub fn explore_cmd(ctx: &Ctx, args: ExploreInput) -> Outcome<ExploreOutcome> {
    let db = open_graph_db(ctx)?;
    let layout = discover_hall(ctx)?;
    let result = explore::explore(
        &db,
        layout.root().as_std_path(),
        &args.query,
        args.repo.as_deref(),
    )
    .map_err(|err| Failure::failed("graph.explore_failed", err.to_string()))?;
    Ok(Report::new(ExploreOutcome(result)))
}

#[derive(Debug, Clone, Serialize)]
pub struct ExploreOutcome(pub ExploreResult);

impl WriteHuman for ExploreOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(w, "Explore results for query `{}`:", res.query)?;
        if res.primary_symbols.is_empty() {
            writeln!(w, "  No symbols found matching query.")?;
        } else {
            writeln!(w, "  Primary Symbols:")?;
            for sym in &res.primary_symbols {
                writeln!(
                    w,
                    "    - {} ({:?}) in {}:{}-{}",
                    sym.symbol.name, sym.symbol.kind, sym.file_path, sym.start_line, sym.end_line
                )?;
                if let Some(sig) = &sym.symbol.signature {
                    writeln!(w, "      Signature: {}", sig)?;
                }
            }
        }
        if !res.call_flows.is_empty() {
            writeln!(w, "  Call Flows:")?;
            for flow in &res.call_flows {
                writeln!(
                    w,
                    "    - {} -> {} ({:?}, line {})",
                    flow.caller, flow.callee, flow.edge_kind, flow.line
                )?;
            }
        }
        if let Some(impact) = &res.impact_summary {
            writeln!(w, "  Impact: {}", impact)?;
        }
        Ok(())
    }
}

// 2. affected
pub fn affected_cmd(ctx: &Ctx, args: AffectedInput) -> Outcome<AffectedOutcome> {
    let db = open_graph_db(ctx)?;
    let mut files = args.files;
    if args.stdin {
        let stdin = io::stdin();
        let stdin_files = parse_files_from_reader(stdin.lock());
        files.extend(stdin_files);
    }
    let max_depth = args.max_depth.unwrap_or(5);
    let result = affected::find_affected_tests(&db, &files, args.repo.as_deref(), max_depth)
        .map_err(|err| Failure::failed("graph.affected_failed", err.to_string()))?;
    Ok(Report::new(AffectedOutcome(result)))
}

#[derive(Debug, Clone, Serialize)]
pub struct AffectedOutcome(pub AffectedResult);

impl WriteHuman for AffectedOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(w, "Changed files ({}):", res.changed_files.len())?;
        for file in &res.changed_files {
            writeln!(w, "  - {}", file)?;
        }
        writeln!(
            w,
            "Affected test files ({}):",
            res.affected_test_files.len()
        )?;
        if res.affected_test_files.is_empty() {
            writeln!(w, "  No affected test files found.")?;
        } else {
            for test in &res.affected_test_files {
                writeln!(w, "  - {}", test)?;
            }
        }
        Ok(())
    }
}

// 3. path
pub fn path_cmd(ctx: &Ctx, args: PathInput) -> Outcome<PathOutcome> {
    let db = open_graph_db(ctx)?;
    let max_hops = args.max_hops.unwrap_or(10);
    let result = path::find_shortest_path(&db, &args.from, &args.to, max_hops)
        .map_err(|err| Failure::failed("graph.path_failed", err.to_string()))?;
    Ok(Report::new(PathOutcome(result)))
}

#[derive(Debug, Clone, Serialize)]
pub struct PathOutcome(pub Option<PathResult>);

impl WriteHuman for PathOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if let Some(res) = &self.0 {
            writeln!(w, "Path from `{}` to `{}`:", res.from, res.to)?;
            if res.steps.is_empty() {
                writeln!(w, "  No path found.")?;
            } else {
                for (idx, step) in res.steps.iter().enumerate() {
                    writeln!(
                        w,
                        "  {}. {} -> {} ({:?})",
                        idx + 1,
                        step.source,
                        step.target,
                        step.edge_kind
                    )?;
                }
            }
        } else {
            writeln!(w, "No path found.")?;
        }
        Ok(())
    }
}

// 4. find
pub fn find_cmd(ctx: &Ctx, args: FindInput) -> Outcome<FindOutcome> {
    let db = open_graph_db(ctx)?;
    let limit = args.limit.unwrap_or(20);
    let symbols = query::find_symbols(&db, &args.query, args.repo.as_deref(), limit)
        .map_err(|err| Failure::failed("graph.find_failed", err.to_string()))?;
    Ok(Report::new(FindOutcome {
        query: args.query,
        symbols,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct FindOutcome {
    pub query: String,
    pub symbols: Vec<query::SymbolLocation>,
}

impl WriteHuman for FindOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Symbols matching `{}` ({} found):",
            self.query,
            self.symbols.len()
        )?;
        for sym in &self.symbols {
            let id_str = sym
                .symbol
                .id
                .map(|id| format!("[#{id}] "))
                .unwrap_or_default();
            writeln!(
                w,
                "  - {}{}:{:?} in {}/{} ({}:{})",
                id_str,
                sym.symbol.name,
                sym.symbol.kind,
                sym.symbol.repo,
                sym.file_path,
                sym.symbol.span.start_line,
                sym.symbol.span.start_col
            )?;
            if let Some(sig) = &sym.symbol.signature {
                writeln!(w, "      Signature: {}", sig)?;
            }
        }
        Ok(())
    }
}

// 5. callers
pub fn callers_cmd(ctx: &Ctx, args: CallersInput) -> Outcome<CallersOutcome> {
    let db = open_graph_db(ctx)?;
    let callers = query::get_callers(
        &db,
        &args.symbol,
        args.repo.as_deref(),
        args.cross_repo,
        args.min_confidence.unwrap_or(0.0),
    )
    .map_err(|err| Failure::failed("graph.callers_failed", err.to_string()))?;
    Ok(Report::new(CallersOutcome {
        symbol: args.symbol,
        callers,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct CallersOutcome {
    pub symbol: String,
    pub callers: Vec<query::CallerInfo>,
}

impl WriteHuman for CallersOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Callers of `{}` ({} found):",
            self.symbol,
            self.callers.len()
        )?;
        for caller in &self.callers {
            writeln!(
                w,
                "  - {} ({:?}) in {}/{} (line {}, confidence: {:.2}, prov: {:?})",
                caller.caller.name,
                caller.caller.kind,
                caller.caller.repo,
                caller.caller_file_path,
                caller.line,
                caller.confidence,
                caller.provenance
            )?;
        }
        Ok(())
    }
}

// 6. callees
pub fn callees_cmd(ctx: &Ctx, args: CalleesInput) -> Outcome<CalleesOutcome> {
    let db = open_graph_db(ctx)?;
    let callees = query::get_callees(&db, args.symbol_id)
        .map_err(|err| Failure::failed("graph.callees_failed", err.to_string()))?;
    Ok(Report::new(CalleesOutcome {
        symbol_id: args.symbol_id,
        callees,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct CalleesOutcome {
    pub symbol_id: i64,
    pub callees: Vec<query::CalleeInfo>,
}

impl WriteHuman for CalleesOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Callees of symbol ID {} ({} found):",
            self.symbol_id,
            self.callees.len()
        )?;
        for callee in &self.callees {
            let target_name = callee
                .callee_symbol
                .as_ref()
                .map(|c| c.name.as_str())
                .unwrap_or(&callee.callee_name);
            let target_kind = callee
                .callee_symbol
                .as_ref()
                .map(|c| format!(" ({:?})", c.kind))
                .unwrap_or_default();
            writeln!(
                w,
                "  - {}{} (line {}, kind: {:?}, prov: {:?})",
                target_name, target_kind, callee.line, callee.edge_kind, callee.provenance
            )?;
        }
        Ok(())
    }
}

// 7. file
pub fn file_cmd(ctx: &Ctx, args: FileInput) -> Outcome<FileOutcome> {
    let db = open_graph_db(ctx)?;
    let outline = query::get_file_outline(&db, &args.repo, &args.path)
        .map_err(|err| Failure::failed("graph.file_failed", err.to_string()))?;
    Ok(Report::new(FileOutcome(outline)))
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOutcome(pub query::FileOutline);

impl WriteHuman for FileOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let outline = &self.0;
        writeln!(
            w,
            "File outline for {}/{} ({} symbols):",
            outline.repo,
            outline.file_path,
            outline.symbols.len()
        )?;
        for sym in &outline.symbols {
            let id_str = sym.id.map(|id| format!("[#{id}] ")).unwrap_or_default();
            let exp = if sym.is_exported { " [pub]" } else { "" };
            writeln!(
                w,
                "  - {}{}: {:?}{} ({}:{})",
                id_str, sym.name, sym.kind, exp, sym.span.start_line, sym.span.start_col
            )?;
            if let Some(sig) = &sym.signature {
                writeln!(w, "      Signature: {}", sig)?;
            }
        }
        Ok(())
    }
}

// 8. index
pub fn index_cmd(ctx: &Ctx, args: IndexInput) -> Outcome<IndexBatchOutcome> {
    let layout = discover_hall(ctx)?;
    let lock_path = layout.ivar_dir().join("memory.lock");
    let lock_file = std::fs::File::create(lock_path.as_std_path()).map_err(|err| {
        Failure::failed(
            "graph.lock_failed",
            format!("Failed to create/open index lock file at {lock_path}: {err}"),
        )
    })?;
    lock_file.lock().map_err(|err| {
        Failure::failed(
            "graph.lock_failed",
            format!("Failed to acquire exclusive lock on {lock_path}: {err}"),
        )
    })?;

    let db = open_graph_db(ctx)?;
    let manifest = read_manifest(&layout)?;

    let mut outcomes = Vec::new();

    if let Some(target_repo) = args.repo {
        let repo_decl = manifest
            .repos()
            .iter()
            .find(|r| r.name().as_str() == target_repo)
            .ok_or_else(|| {
                Failure::blocked(
                    "repo.not_found",
                    format!("Repository `{target_repo}` not found in manifest"),
                )
            })?;
        let repo_path = layout.repo_worktree(repo_decl.name(), repo_decl.default_branch());
        if !Path::new(repo_path.as_std_path()).exists() {
            return Err(Failure::blocked(
                "repo.worktree_missing",
                format!(
                    "Repository worktree does not exist at `{repo_path}`. Run `ivar sync` first."
                ),
            ));
        }
        let outcome = index::index_repo(
            &db,
            repo_decl.name().as_str(),
            repo_path.as_std_path(),
            args.full,
        )
        .map_err(|err| Failure::failed("graph.index_failed", err.to_string()))?;
        outcomes.push(outcome);
    } else {
        for repo_decl in manifest.repos() {
            let repo_path = layout.repo_worktree(repo_decl.name(), repo_decl.default_branch());
            if Path::new(repo_path.as_std_path()).exists() {
                let outcome = index::index_repo(
                    &db,
                    repo_decl.name().as_str(),
                    repo_path.as_std_path(),
                    args.full,
                )
                .map_err(|err| Failure::failed("graph.index_failed", err.to_string()))?;
                outcomes.push(outcome);
            }
        }
    }

    // Relink only after the graph changed; dirty unsupported files must remain a no-op.
    let should_link = outcomes
        .iter()
        .any(|outcome| outcome.files_indexed > 0 || outcome.files_deleted > 0);
    let cross_edges_linked = if should_link {
        let cross_edges = cross_repo::link_cross_repo_edges(&db)
            .map_err(|err| Failure::failed("graph.cross_repo_failed", err.to_string()))?;
        cross_edges.total_linked
    } else {
        0
    };

    Ok(Report::new(IndexBatchOutcome {
        repos: outcomes,
        cross_edges_linked,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexBatchOutcome {
    pub repos: Vec<index::IndexOutcome>,
    pub cross_edges_linked: usize,
}

impl WriteHuman for IndexBatchOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Codebase Graph Indexing Complete:")?;
        for outcome in &self.repos {
            if outcome.skipped_up_to_date {
                writeln!(
                    w,
                    "  - {}: up to date (took {}ms)",
                    outcome.repo, outcome.duration_ms
                )?;
            } else {
                writeln!(
                    w,
                    "  - {}: {} files indexed, {} deleted, {} symbols, {} edges (took {}ms)",
                    outcome.repo,
                    outcome.files_indexed,
                    outcome.files_deleted,
                    outcome.symbols_indexed,
                    outcome.edges_indexed,
                    outcome.duration_ms
                )?;
            }
        }
        if self.cross_edges_linked > 0 {
            writeln!(w, "  Linked {} cross-repo edges.", self.cross_edges_linked)?;
        }
        Ok(())
    }
}

// 9. stats
pub fn stats_cmd(ctx: &Ctx) -> Outcome<StatsOutcome> {
    let db = open_graph_db(ctx)?;
    let stats = query::get_graph_stats(&db)
        .map_err(|err| Failure::failed("graph.stats_failed", err.to_string()))?;
    Ok(Report::new(StatsOutcome(stats)))
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsOutcome(pub GraphStats);

impl WriteHuman for StatsOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let s = &self.0;
        writeln!(w, "Codebase Graph Statistics:")?;
        writeln!(w, "  Repositories: {}", s.repo_count)?;
        writeln!(w, "  Files:        {}", s.file_count)?;
        writeln!(w, "  Symbols:      {}", s.symbol_count)?;
        writeln!(w, "  Edges:        {}", s.edge_count)?;
        writeln!(w, "  DB Size:      {} bytes", s.db_size_bytes)?;
        Ok(())
    }
}

// 10. impact
pub fn impact_cmd(ctx: &Ctx, args: ImpactInput) -> Outcome<ImpactOutcome> {
    let db = open_graph_db(ctx)?;
    let max_depth = args.max_depth.unwrap_or(5);
    let result = query::get_impact(&db, args.symbol_id, max_depth)
        .map_err(|err| Failure::failed("graph.impact_failed", err.to_string()))?;
    Ok(Report::new(ImpactOutcome(result)))
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactOutcome(pub query::ImpactResult);

impl WriteHuman for ImpactOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let res = &self.0;
        writeln!(
            w,
            "Blast Radius Impact Analysis for symbol `{}` (ID: {}):",
            res.root_symbol.name,
            res.root_symbol.id.unwrap_or(0)
        )?;
        writeln!(w, "  Total affected symbols: {}", res.total_affected)?;
        for item in &res.affected_symbols {
            writeln!(
                w,
                "  - [depth {}] {} ({:?}) in {}/{} (via {:?})",
                item.depth,
                item.symbol.name,
                item.symbol.kind,
                item.symbol.repo,
                item.file_path,
                item.path_via
            )?;
        }
        Ok(())
    }
}

// 11. mcp
pub fn mcp_cmd(ctx: &Ctx) -> Outcome<McpOutcome> {
    let db = open_graph_db(ctx)?;
    let layout = discover_hall(ctx)?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stdin_lock = stdin.lock();
    let stdout_lock = stdout.lock();
    mcp::run_mcp_server(
        &db,
        Some(layout.root().as_std_path()),
        stdin_lock,
        stdout_lock,
        |repo| {
            let report = index_cmd(
                ctx,
                IndexInput {
                    repo: repo.map(str::to_owned),
                    full: false,
                },
            )
            .map_err(|err| err.to_string())?;
            serde_json::to_value(report.value).map_err(|err| err.to_string())
        },
    )
    .map_err(|err| Failure::failed("graph.mcp_failed", err.to_string()))?;
    Ok(Report::new(McpOutcome))
}

#[derive(Debug, Clone, Serialize)]
pub struct McpOutcome;

impl WriteHuman for McpOutcome {
    fn write_human(&self, _w: &mut impl io::Write) -> io::Result<()> {
        Ok(())
    }
}
