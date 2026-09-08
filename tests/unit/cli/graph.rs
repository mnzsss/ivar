#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::Parser;

use crate::cli::graph::*;
use crate::cli::root::{Cli, Command};

#[test]
fn test_cli_graph_explore_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "explore", "init_hall"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Explore(args)) => {
            assert_eq!(args.query, "init_hall");
            assert_eq!(args.repo, None);
        }
        other => panic!("expected graph explore, got {other:?}"),
    }

    let cli_with_repo = Cli::try_parse_from(["ivar", "graph", "explore", "init_hall", "--repo", "my-repo"]).unwrap();
    match cli_with_repo.command {
        Command::Graph(GraphCommand::Explore(args)) => {
            assert_eq!(args.query, "init_hall");
            assert_eq!(args.repo.as_deref(), Some("my-repo"));
        }
        other => panic!("expected graph explore, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_affected_parsing() {
    let cli = Cli::try_parse_from([
        "ivar", "graph", "affected", "src/foo.rs", "src/bar.rs", "--stdin", "--repo", "my-repo", "--max-depth", "5",
    ])
    .unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Affected(args)) => {
            assert_eq!(args.files, vec!["src/foo.rs", "src/bar.rs"]);
            assert!(args.stdin);
            assert_eq!(args.repo.as_deref(), Some("my-repo"));
            assert_eq!(args.max_depth, Some(5));
        }
        other => panic!("expected graph affected, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_path_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "path", "from_sym", "to_sym", "--max-hops", "7"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Path(args)) => {
            assert_eq!(args.from, "from_sym");
            assert_eq!(args.to, "to_sym");
            assert_eq!(args.max_hops, Some(7));
        }
        other => panic!("expected graph path, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_find_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "find", "handle_*", "--repo", "core", "--limit", "20"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Find(args)) => {
            assert_eq!(args.query, "handle_*");
            assert_eq!(args.repo.as_deref(), Some("core"));
            assert_eq!(args.limit, Some(20));
        }
        other => panic!("expected graph find, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_callers_parsing() {
    let cli = Cli::try_parse_from([
        "ivar", "graph", "callers", "target_fn", "--repo", "core", "--cross-repo", "--min-confidence", "0.85",
    ])
    .unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Callers(args)) => {
            assert_eq!(args.symbol, "target_fn");
            assert_eq!(args.repo.as_deref(), Some("core"));
            assert!(args.cross_repo);
            assert_eq!(args.min_confidence, Some(0.85));
        }
        other => panic!("expected graph callers, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_callees_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "callees", "42"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Callees(args)) => {
            assert_eq!(args.symbol_id, 42);
        }
        other => panic!("expected graph callees, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_file_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "file", "core-repo", "src/main.rs"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::File(args)) => {
            assert_eq!(args.repo, "core-repo");
            assert_eq!(args.path, "src/main.rs");
        }
        other => panic!("expected graph file, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_index_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "index", "--repo", "my-repo", "--full"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Index(args)) => {
            assert_eq!(args.repo.as_deref(), Some("my-repo"));
            assert!(args.full);
        }
        other => panic!("expected graph index, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_stats_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "stats"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Stats) => {}
        other => panic!("expected graph stats, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_impact_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "impact", "99", "--max-depth", "3"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Impact(args)) => {
            assert_eq!(args.symbol_id, 99);
            assert_eq!(args.max_depth, Some(3));
        }
        other => panic!("expected graph impact, got {other:?}"),
    }
}

#[test]
fn test_cli_graph_mcp_parsing() {
    let cli = Cli::try_parse_from(["ivar", "graph", "mcp"]).unwrap();
    match cli.command {
        Command::Graph(GraphCommand::Mcp) => {}
        other => panic!("expected graph mcp, got {other:?}"),
    }
}
