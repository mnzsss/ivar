#![allow(clippy::unwrap_used, clippy::panic)]

use clap::Parser;

use crate::action::memory::review::Verdict;
use crate::cli::memory::{HandoffCommand, MemoryCommand};
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

#[test]
fn memory_lint_parses_with_json() {
    let cli = Cli::try_parse_from(["ivar", "memory", "lint", "--json"]).unwrap();
    assert!(matches!(cli.command, Command::Memory(MemoryCommand::Lint)));
}

#[test]
fn memory_review_parses_with_and_without_a_feature() {
    let named = Cli::try_parse_from(["ivar", "memory", "review", "checkout", "--json"]).unwrap();
    let Command::Memory(MemoryCommand::Review { feature, .. }) = named.command else {
        panic!("expected memory review");
    };
    assert_eq!(feature.as_deref(), Some("checkout"));
    assert!(named.json);

    let bare = Cli::try_parse_from(["ivar", "memory", "review"]).unwrap();
    assert!(matches!(
        bare.command,
        Command::Memory(MemoryCommand::Review {
            feature: None,
            confirm: false,
            ..
        })
    ));
}

#[test]
fn memory_review_without_confirm_is_read_only() {
    let cli = Cli::try_parse_from(["ivar", "memory", "review", "ship"]).unwrap();
    let Command::Memory(MemoryCommand::Review {
        confirm,
        stale,
        not_required,
        ..
    }) = cli.command
    else {
        panic!("expected memory review");
    };
    assert!(!confirm);
    assert!(stale.is_empty());
    assert!(not_required.is_none());
}

#[test]
fn memory_review_confirm_parses_verdicts_split_at_the_last_equals() {
    let cli = Cli::try_parse_from([
        "ivar",
        "memory",
        "review",
        "ship",
        "--confirm",
        "--stale",
        "docs/memory/gotcha/a.md=updated",
        "--stale",
        "docs/memory/gotcha/b=c.md=still_valid",
    ])
    .unwrap();
    let Command::Memory(MemoryCommand::Review {
        confirm,
        stale,
        not_required,
        ..
    }) = cli.command
    else {
        panic!("expected memory review");
    };
    assert!(confirm);
    assert!(not_required.is_none());
    assert_eq!(
        stale,
        [
            ("docs/memory/gotcha/a.md".to_owned(), Verdict::Updated),
            ("docs/memory/gotcha/b=c.md".to_owned(), Verdict::StillValid),
        ]
    );

    let cli = Cli::try_parse_from([
        "ivar",
        "memory",
        "review",
        "--confirm",
        "--not-required",
        "internal refactor",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Memory(MemoryCommand::Review { not_required: Some(reason), .. })
            if reason == "internal refactor"
    ));
}

#[test]
fn memory_review_stale_and_not_required_need_confirm() {
    for args in [
        ["--stale", "p.md=updated"],
        ["--not-required", "internal refactor"],
    ] {
        let error =
            Cli::try_parse_from(["ivar", "memory", "review", "ship"].into_iter().chain(args))
                .unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}

#[test]
fn memory_review_stale_rejects_a_malformed_verdict() {
    for raw in ["no-equals", "=updated", "p.md=maybe", "p.md="] {
        assert!(
            Cli::try_parse_from(["ivar", "memory", "review", "--confirm", "--stale", raw]).is_err(),
            "{raw}"
        );
    }
}

#[test]
fn memory_handoff_begin_parses_repeated_lists_and_an_address() {
    let cli = Cli::try_parse_from([
        "ivar",
        "memory",
        "handoff",
        "begin",
        "--summary",
        "half done",
        "--pending",
        "write docs",
        "--pending",
        "run clippy",
        "--decision",
        "keep the sweep",
        "--file",
        "src/a.rs",
        "--hall",
        "--to-provider",
        "omp",
        "--json",
    ])
    .unwrap();
    let Command::Memory(MemoryCommand::Handoff {
        command:
            HandoffCommand::Begin {
                summary,
                pending,
                decision,
                file,
                feature,
                hall,
                to_provider,
                to_session,
                session,
            },
    }) = cli.command
    else {
        panic!("expected memory handoff begin");
    };
    assert_eq!(summary, "half done");
    assert_eq!(pending, vec!["write docs", "run clippy"]);
    assert_eq!(decision, vec!["keep the sweep"]);
    assert_eq!(file, vec!["src/a.rs"]);
    assert!(hall && feature.is_none());
    assert_eq!(to_provider.as_deref(), Some("omp"));
    assert!(to_session.is_none() && session.is_none());
    assert!(cli.json);
}

#[test]
fn memory_handoff_begin_refuses_feature_with_hall_and_two_addresses() {
    assert!(
        Cli::try_parse_from([
            "ivar",
            "memory",
            "handoff",
            "begin",
            "--summary",
            "s",
            "--feature",
            "x",
            "--hall"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "ivar",
            "memory",
            "handoff",
            "begin",
            "--summary",
            "s",
            "--to-provider",
            "omp",
            "--to-session",
            "abc"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["ivar", "memory", "handoff", "begin"]).is_err());
}

#[test]
fn memory_handoff_list_and_cancel_parse() {
    let list =
        Cli::try_parse_from(["ivar", "memory", "handoff", "list", "--feature", "login"]).unwrap();
    assert!(matches!(
        list.command,
        Command::Memory(MemoryCommand::Handoff {
            command: HandoffCommand::List {
                feature: Some(_),
                hall: false
            }
        })
    ));
    let cancel = Cli::try_parse_from(["ivar", "memory", "handoff", "cancel", "h-1"]).unwrap();
    let Command::Memory(MemoryCommand::Handoff {
        command: HandoffCommand::Cancel { id },
    }) = cancel.command
    else {
        panic!("expected memory handoff cancel");
    };
    assert_eq!(id, "h-1");
}

#[test]
fn memory_brief_parses_with_and_without_a_provider() {
    let cli = Cli::try_parse_from(["ivar", "memory", "brief", "--provider", "omp"]).unwrap();
    let Command::Memory(MemoryCommand::Brief { provider }) = cli.command else {
        panic!("expected memory brief");
    };
    assert_eq!(provider.as_deref(), Some("omp"));
    assert!(matches!(
        Cli::try_parse_from(["ivar", "memory", "brief"])
            .unwrap()
            .command,
        Command::Memory(MemoryCommand::Brief { provider: None })
    ));
}
