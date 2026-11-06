#![allow(clippy::unwrap_used, clippy::panic)]

use clap::Parser;

use crate::action::feature::deliver::DeliverInput;
use crate::cli::root::{Cli, Command, FeatureCommand};

#[test]
fn skip_memory_review_takes_a_reason_and_reaches_the_input() {
    let cli = Cli::try_parse_from([
        "ivar",
        "feature",
        "deliver",
        "checkout",
        "--fingerprint",
        "abc",
        "--skip-memory-review",
        "hotfix for prod",
    ])
    .unwrap();
    let Command::Feature(FeatureCommand::Deliver(args)) = cli.command else {
        panic!("expected feature deliver");
    };

    assert_eq!(args.skip_memory_review.as_deref(), Some("hotfix for prod"));
    assert_eq!(
        DeliverInput::from(args).skip_memory_review.as_deref(),
        Some("hotfix for prod")
    );
}

#[test]
fn skip_memory_review_without_a_value_is_a_parse_error() {
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "deliver",
            "checkout",
            "--skip-memory-review"
        ])
        .is_err()
    );
}
