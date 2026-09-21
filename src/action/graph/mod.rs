pub mod affected;
pub mod clean;
pub mod compact;
pub mod complexity;
pub mod cross_repo;
pub mod dead_code;
pub mod explore;
pub mod freshness;
pub mod hierarchy;
pub mod index;
pub mod input;
pub mod layer;
pub mod mcp;
pub mod mcp_registration;
pub mod narrate;
pub mod outcome;
pub mod path;
pub mod query;
pub mod session;
pub mod usage;
pub mod view;
pub mod viz;

use std::io;
use std::path::Path;

use crate::action::Ctx;
use crate::action::discover_hall;
use crate::action::read_manifest;
use crate::domain::graph::MissKind;
use crate::error::{Failure, Outcome, Report};
use crate::infra::progress::Progress;
use crate::store::graph::db::GraphDb;
use crate::store::graph::db::usage::MissFilter;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

pub use affected::{AffectedError, find_affected_tests, is_test_file, parse_files_from_reader};
pub use clean::clean_cmd;
pub use compact::ToCompact;
pub use complexity::{ComplexityError, execute_complexity};
pub use cross_repo::{CrossRepoLinkOutcome, link_cross_repo_edges};
pub use dead_code::{DeadCodeError, execute_dead_code};
pub use explore::{ExploreError, explore};
pub use hierarchy::{HierarchyError, execute_hierarchy};
pub use index::{HallIndex, IndexOutcome, index_hall, index_repo};
pub use input::*;
pub use mcp::run_mcp_server;
pub use outcome::*;
pub use path::{PathError, find_shortest_path};
pub use query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactItem, ImpactResult, QueryError, SymbolLocation,
    find_symbols, get_callees, get_callers, get_file_outline, get_graph_stats, get_impact,
};
pub use usage::{ResultCount, record_usage};
pub use view::lifecycle::{
    ViewSession, execute_view_session, launch_browser, prepare_view_session,
};
pub use viz::{VizError, execute_viz};
fn open_graph_db(ctx: &Ctx) -> Result<GraphDb, Failure> {
    let layout = discover_hall(ctx)?;
    let db_path = layout.ivar_dir().join("memory.db");
    let db = GraphDb::open(db_path.as_std_path()).map_err(|err| {
        Failure::failed(
            "graph.db_open_failed",
            format!("Failed to open graph database at {db_path}: {err}"),
        )
    })?;
    let view = session::resolve_session_view(&layout, &ctx.cwd)?;
    freshness::ensure_session_freshness(&db, &layout, &view)?;
    Ok(db)
}

// 1. explore
pub fn explore_cmd(ctx: &Ctx, args: &ExploreInput) -> Outcome<ExploreOutcome> {
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

// 2. affected
pub fn affected_cmd(ctx: &Ctx, args: AffectedInput) -> Outcome<AffectedOutcome> {
    let db = open_graph_db(ctx)?;
    let layout = discover_hall(ctx).ok();
    let mut files = args.files;
    if args.stdin {
        let stdin = io::stdin();
        let stdin_files = parse_files_from_reader(stdin.lock());
        files.extend(stdin_files);
    }
    let max_depth = args.max_depth.unwrap_or(5);
    let root = layout.as_ref().map(|l| l.root().as_std_path());
    let result =
        affected::find_affected_tests_with_root(&db, root, &files, args.repo.as_deref(), max_depth)
            .map_err(|err| Failure::failed("graph.affected_failed", err.to_string()))?;
    Ok(Report::new(AffectedOutcome(result)))
}

// 3. path
pub fn path_cmd(ctx: &Ctx, args: &PathInput) -> Outcome<PathOutcome> {
    let db = open_graph_db(ctx)?;
    let max_hops = args.max_hops.unwrap_or(10);
    let result = path::find_shortest_path(&db, &args.from, &args.to, max_hops)
        .map_err(|err| Failure::failed("graph.path_failed", err.to_string()))?;
    Ok(Report::new(PathOutcome(result)))
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

// 6. callees
pub fn callees_cmd(ctx: &Ctx, args: &CalleesInput) -> Outcome<CalleesOutcome> {
    let db = open_graph_db(ctx)?;
    let callees = query::get_callees(&db, args.symbol_id)
        .map_err(|err| Failure::failed("graph.callees_failed", err.to_string()))?;
    Ok(Report::new(CalleesOutcome {
        symbol_id: args.symbol_id,
        callees,
    }))
}

// 7. file
pub fn file_cmd(ctx: &Ctx, args: &FileInput) -> Outcome<FileOutcome> {
    let db = open_graph_db(ctx)?;
    let outline = query::get_file_outline(&db, &args.repo, &args.path)
        .map_err(|err| Failure::failed("graph.file_failed", err.to_string()))?;
    Ok(Report::new(FileOutcome(outline)))
}

fn lock_index(layout: &Layout) -> Result<std::fs::File, Failure> {
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
    Ok(lock_file)
}

/// Incrementally reindexes the base repositories of a hall that already has a
/// graph database. Returns `None` when the hall never built one.
pub fn refresh_base_graph(
    layout: &Layout,
    manifest: &Manifest,
    progress: &dyn Progress,
) -> Result<Option<HallIndex>, Failure> {
    let db_path = layout.ivar_dir().join("memory.db");
    if !db_path.as_std_path().exists() {
        return Ok(None);
    }
    let _lock = lock_index(layout)?;
    let db = GraphDb::open(db_path.as_std_path()).map_err(|err| {
        Failure::failed(
            "graph.db_open_failed",
            format!("Failed to open graph database at {db_path}: {err}"),
        )
    })?;
    let hall = index::index_hall(&db, layout, manifest, false, progress);
    if hall
        .repos
        .iter()
        .any(|outcome| outcome.files_indexed > 0 || outcome.files_deleted > 0)
    {
        cross_repo::link_cross_repo_edges(&db)
            .map_err(|err| Failure::failed("graph.cross_repo_failed", err.to_string()))?;
    }
    Ok(Some(hall))
}

// 8. index
pub fn index_cmd(ctx: &Ctx, args: IndexInput) -> Outcome<IndexBatchOutcome> {
    let layout = discover_hall(ctx)?;
    let _lock = lock_index(&layout)?;

    let db = open_graph_db(ctx)?;
    let manifest = read_manifest(&layout)?;

    let hall = if let Some(target_repo) = args.repo {
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
            ctx.progress(),
        )
        .map_err(|err| Failure::failed("graph.index_failed", err.to_string()))?;
        HallIndex {
            repos: vec![outcome],
            repos_failed: Vec::new(),
        }
    } else {
        index::index_hall(&db, &layout, &manifest, args.full, ctx.progress())
    };

    // Relink only after the graph changed; dirty unsupported files must remain a no-op.
    let should_link = hall
        .repos
        .iter()
        .any(|outcome| outcome.files_indexed > 0 || outcome.files_deleted > 0);
    let cross_edges_linked = if should_link {
        let cross_edges = cross_repo::link_cross_repo_edges(&db)
            .map_err(|err| Failure::failed("graph.cross_repo_failed", err.to_string()))?;
        cross_edges.total_linked
    } else {
        0
    };

    let (mcp_registration, warning) = if args.register_mcp {
        mcp_registration::register_graph_mcp(&layout, &manifest)
    } else {
        (McpRegistration::Skipped, None)
    };

    Ok(Report::with_warnings(
        IndexBatchOutcome {
            repos: hall.repos,
            repos_failed: hall.repos_failed,
            cross_edges_linked,
            mcp_registration,
            next_command: (mcp_registration == McpRegistration::Registered).then_some("ivar sync"),
        },
        warning.into_iter().collect(),
    ))
}

// 9. stats
pub fn stats_cmd(ctx: &Ctx) -> Outcome<StatsOutcome> {
    let db = open_graph_db(ctx)?;
    let _ = db.prune(MISS_RETENTION_DAYS);
    let stats = query::get_graph_stats(&db)
        .map_err(|err| Failure::failed("graph.stats_failed", err.to_string()))?;
    Ok(Report::new(StatsOutcome(stats)))
}

pub(crate) const MISS_RETENTION_DAYS: i64 = 30;

fn parse_since(raw: &str, now: i64) -> Result<i64, Failure> {
    if let Ok(ts) = raw.parse::<i64>() {
        return Ok(ts);
    }
    let invalid = || {
        Failure::blocked(
            "graph.since_invalid",
            format!("invalid --since value `{raw}`: use a Unix timestamp, or `Nm`/`Nh`/`Nd`"),
        )
    };
    let (digits, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n: i64 = digits.parse().map_err(|_| invalid())?;
    let unit_secs = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return Err(invalid()),
    };
    Ok(now.saturating_sub(n.saturating_mul(unit_secs)))
}

pub fn misses_cmd(ctx: &Ctx, input: &MissesInput) -> Outcome<MissesOutcome> {
    let kind = input
        .kind
        .as_deref()
        .map(str::parse::<MissKind>)
        .transpose()
        .map_err(|err| Failure::blocked("graph.kind_invalid", err.to_string()))?;
    let since = input
        .since
        .as_deref()
        .map(|raw| parse_since(raw, crate::store::graph::db::types::now_timestamp()))
        .transpose()?;
    let db = open_graph_db(ctx)?;
    let _ = db.prune(MISS_RETENTION_DAYS);
    let misses = db
        .list_misses(&MissFilter { kind, since })
        .map_err(|err| Failure::failed("graph.misses_failed", err.to_string()))?;
    Ok(Report::new(MissesOutcome { misses }))
}

// 10. impact
pub fn impact_cmd(ctx: &Ctx, args: &ImpactInput) -> Outcome<ImpactOutcome> {
    let db = open_graph_db(ctx)?;
    let max_depth = args.max_depth.unwrap_or(5);
    let result = query::get_impact(&db, args.symbol_id, max_depth)
        .map_err(|err| Failure::failed("graph.impact_failed", err.to_string()))?;
    Ok(Report::new(ImpactOutcome(result)))
}

// 11. mcp
pub fn mcp_cmd(ctx: &Ctx, tools: mcp::ToolSurface) -> Outcome<McpOutcome> {
    let db = open_graph_db(ctx)?;
    let layout = discover_hall(ctx)?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stdin_lock = stdin.lock();
    let stdout_lock = stdout.lock();
    mcp::run_mcp_server_with_tools(
        &db,
        Some(layout.root().as_std_path()),
        tools,
        stdin_lock,
        stdout_lock,
        |repo| {
            let report = index_cmd(
                ctx,
                IndexInput {
                    repo: repo.map(str::to_owned),
                    full: false,
                    register_mcp: false,
                },
            )
            .map_err(|err| err.to_string())?;
            serde_json::to_value(report.value).map_err(|err| err.to_string())
        },
    )
    .map_err(|err| Failure::failed("graph.mcp_failed", err.to_string()))?;
    Ok(Report::new(McpOutcome))
}

// 12. dead-code
pub fn dead_code_cmd(ctx: &Ctx, args: &DeadCodeInput) -> Outcome<DeadCodeOutcome> {
    let db = open_graph_db(ctx)?;
    let limit = args.limit.unwrap_or(50);
    let items = dead_code::execute_dead_code(&db, args.repo.as_deref(), limit)
        .map_err(|err| Failure::failed("graph.dead_code_failed", err.to_string()))?;
    Ok(Report::new(DeadCodeOutcome(items)))
}

// 13. complexity
pub fn complexity_cmd(ctx: &Ctx, args: &ComplexityInput) -> Outcome<ComplexityOutcome> {
    let db = open_graph_db(ctx)?;
    let limit = args.limit.unwrap_or(50);
    let items = complexity::execute_complexity(&db, args.repo.as_deref(), args.threshold, limit)
        .map_err(|err| Failure::failed("graph.complexity_failed", err.to_string()))?;
    Ok(Report::new(ComplexityOutcome(items)))
}

// 14. hierarchy
pub fn hierarchy_cmd(ctx: &Ctx, args: &HierarchyInput) -> Outcome<HierarchyOutcome> {
    let db = open_graph_db(ctx)?;
    let item = hierarchy::execute_hierarchy(&db, &args.symbol, args.repo.as_deref())
        .map_err(|err| Failure::failed("graph.hierarchy_failed", err.to_string()))?;
    Ok(Report::new(HierarchyOutcome(item)))
}

// 15. viz
pub fn viz_cmd(ctx: &Ctx, args: &VizInput) -> Outcome<VizOutcome> {
    let db = open_graph_db(ctx)?;
    let output_path = std::path::PathBuf::from(&args.output);
    let (data, resolved_path) = viz::execute_viz(&db, &output_path, args.repo.as_deref())
        .map_err(|err| Failure::failed("graph.viz_failed", err.to_string()))?;
    Ok(Report::new(VizOutcome {
        output_path: resolved_path,
        node_count: data.nodes.len(),
        edge_count: data.edges.len(),
    }))
}

// 16. view
pub fn view_cmd(ctx: &Ctx, input: GraphViewInput) -> Outcome<ViewSession> {
    let db = open_graph_db(ctx)?;

    let session = prepare_view_session(db, input).map_err(|err| {
        Failure::failed(
            "graph.view_failed",
            format!("Failed to start graph viewer: {err}"),
        )
    })?;

    Ok(Report::new(session))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/explore_source.rs"]
mod explore_source_tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/clean_stats.rs"]
mod clean_stats_tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/misses_cmd.rs"]
mod misses_cmd_tests;
