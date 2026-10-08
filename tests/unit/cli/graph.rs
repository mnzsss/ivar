#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::Parser;
use rstest::rstest;

use crate::action::graph::mcp::ToolSurface;
use crate::cli::graph::*;
use crate::cli::root::{Cli, Command};

#[rstest]
#[case::mcp_default_tools(&["ivar", "graph", "mcp"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Mcp(args)) if args.tools == ToolSurface::Explore))]
#[case::mcp_all_tools(&["ivar", "graph", "mcp", "--tools", "all"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Mcp(args)) if args.tools == ToolSurface::All))]
#[case::explore(&["ivar", "graph", "explore", "init_hall"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Explore(args)) if args.query == "init_hall" && args.repo.is_none()))]
#[case::explore_with_repo(&["ivar", "graph", "explore", "init_hall", "--repo", "my-repo"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Explore(args)) if args.query == "init_hall" && args.repo.as_deref() == Some("my-repo")))]
#[case::affected(&["ivar", "graph", "affected", "src/foo.rs", "src/bar.rs", "--stdin", "--repo", "my-repo", "--max-depth", "5"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Affected(args)) if args.files == ["src/foo.rs", "src/bar.rs"] && args.stdin && args.repo.as_deref() == Some("my-repo") && args.max_depth == Some(5)))]
#[case::path(&["ivar", "graph", "path", "from_sym", "to_sym", "--max-hops", "7"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Path(args)) if args.from == "from_sym" && args.to == "to_sym" && args.max_hops == Some(7)))]
#[case::find(&["ivar", "graph", "find", "handle_*", "--repo", "core", "--limit", "20"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Find(args)) if args.query == "handle_*" && args.repo.as_deref() == Some("core") && args.limit == Some(20)))]
#[case::callers(&["ivar", "graph", "callers", "target_fn", "--repo", "core", "--cross-repo", "--min-confidence", "0.85"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Callers(args)) if args.symbol == "target_fn" && args.repo.as_deref() == Some("core") && args.cross_repo && args.min_confidence == Some(0.85)))]
#[case::callees(&["ivar", "graph", "callees", "42"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Callees(args)) if args.symbol_id == 42))]
#[case::file(&["ivar", "graph", "file", "core-repo", "src/main.rs"], |command: &Command| matches!(command, Command::Graph(GraphCommand::File(args)) if args.repo == "core-repo" && args.path == "src/main.rs"))]
#[case::index(&["ivar", "graph", "index", "--repo", "my-repo", "--full"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Index(args)) if args.repo.as_deref() == Some("my-repo") && args.full))]
#[case::stats(&["ivar", "graph", "stats"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Stats)))]
#[case::impact(&["ivar", "graph", "impact", "99", "--max-depth", "3"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Impact(args)) if args.symbol_id == 99 && args.max_depth == Some(3)))]
#[case::view_defaults(&["ivar", "graph", "view"], |command: &Command| matches!(command, Command::Graph(GraphCommand::View(args)) if args.repo.is_none() && args.symbol.is_none() && args.file.is_none() && args.impact.is_none() && args.depth.is_none() && args.limit.is_none() && !args.no_open))]
#[case::view_with_seed_and_bounds(&["ivar", "graph", "view", "--symbol", "explore_query", "--depth", "2", "--limit", "300", "--no-open"], |command: &Command| matches!(command, Command::Graph(GraphCommand::View(args)) if args.symbol.as_deref() == Some("explore_query") && args.depth == Some(2) && args.limit == Some(300) && args.no_open))]
#[case::clean_one_repo(&["ivar", "graph", "clean", "--repo", "my-repo"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Clean(args)) if args.repo.as_deref() == Some("my-repo") && !args.all))]
#[case::clean_all(&["ivar", "graph", "clean", "--all"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Clean(args)) if args.repo.is_none() && args.all))]
#[case::misses_filtered(&["ivar", "graph", "misses", "--kind", "followup", "--since", "7d", "--json"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Misses(args)) if args.kind.as_deref() == Some("followup") && args.since.as_deref() == Some("7d")))]
#[case::misses_defaults(&["ivar", "graph", "misses"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Misses(args)) if args.kind.is_none() && args.since.is_none() && args.limit == 10))]
#[case::misses_limit_25(&["ivar", "graph", "misses", "--limit", "25"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Misses(args)) if args.limit == 25))]
#[case::misses_limit_0(&["ivar", "graph", "misses", "--limit", "0"], |command: &Command| matches!(command, Command::Graph(GraphCommand::Misses(args)) if args.limit == 0))]
fn graph_cli_parses_argv_into_command(
    #[case] argv: &[&str],
    #[case] expected: fn(&Command) -> bool,
) {
    let cli = Cli::try_parse_from(argv).unwrap_or_else(|error| panic!("{argv:?} refused: {error}"));

    assert!(
        expected(&cli.command),
        "{argv:?} parsed as {:?}",
        cli.command
    );
}

#[test]
fn test_cli_graph_view_mutually_exclusive_seeds() {
    // Cannot pass both --repo and --symbol
    let err = Cli::try_parse_from(["ivar", "graph", "view", "--repo", "ivar", "--symbol", "foo"]);
    assert!(err.is_err(), "repo and symbol must conflict");

    // Cannot pass both --symbol and --file
    let err = Cli::try_parse_from([
        "ivar",
        "graph",
        "view",
        "--symbol",
        "foo",
        "--file",
        "src/lib.rs",
    ]);
    assert!(err.is_err(), "symbol and file must conflict");

    // Cannot pass both --file and --impact
    let err = Cli::try_parse_from([
        "ivar",
        "graph",
        "view",
        "--file",
        "src/lib.rs",
        "--impact",
        "bar",
    ]);
    assert!(err.is_err(), "file and impact must conflict");
}
