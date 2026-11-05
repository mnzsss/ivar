#![allow(clippy::unwrap_used)]

use clap::Parser;

use crate::cli::memory::MemoryCommand;
use crate::cli::root::{Cli, Command};

#[test]
fn memory_status_parses_with_json() {
    let cli = Cli::try_parse_from(["ivar", "memory", "status", "--json"]).unwrap();

    assert!(matches!(
        cli.command,
        Command::Memory(MemoryCommand::Status)
    ));
    assert!(cli.json);
}

#[test]
fn memory_clean_parses() {
    let cli = Cli::try_parse_from(["ivar", "memory", "clean"]).unwrap();

    assert!(matches!(cli.command, Command::Memory(MemoryCommand::Clean)));
}

#[test]
fn memory_query_parses_filters() {
    let cli = Cli::try_parse_from([
        "ivar",
        "memory",
        "query",
        "worktree teardown",
        "--kind",
        "product",
        "--kind",
        "gotcha",
        "--tier",
        "semantic",
        "--tag",
        "fts",
        "--limit",
        "5",
        "--include-expired",
        "--explain",
        "--json",
    ])
    .unwrap();
    let Command::Memory(MemoryCommand::Query {
        text,
        kind,
        tier,
        tag,
        limit,
        include_expired,
        explain,
    }) = cli.command
    else {
        panic!("not a query");
    };
    assert_eq!(text, "worktree teardown");
    assert_eq!(kind, ["product", "gotcha"]);
    assert_eq!(
        (tier.as_deref(), tag.as_deref(), limit),
        (Some("semantic"), Some("fts"), 5)
    );
    assert!(include_expired && explain && cli.json);
}

#[test]
fn memory_query_defaults_the_limit_and_reindex_parses() {
    let cli = Cli::try_parse_from(["ivar", "memory", "query", "x"]).unwrap();
    assert!(matches!(
        cli.command,
        Command::Memory(MemoryCommand::Query { limit: 10, .. })
    ));
    let cli = Cli::try_parse_from(["ivar", "memory", "reindex"]).unwrap();
    assert!(matches!(
        cli.command,
        Command::Memory(MemoryCommand::Reindex)
    ));
}
