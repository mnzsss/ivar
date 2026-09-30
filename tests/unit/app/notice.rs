#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use clap::Parser as _;

use super::*;
use crate::cli::root::Cli;

fn command(args: &[&str]) -> Command {
    Cli::try_parse_from(std::iter::once("ivar").chain(args.iter().copied()))
        .unwrap_or_else(|e| panic!("{args:?}: {e}"))
        .command
}

#[test]
fn hook_mcp_credential_and_upgrade_verbs_are_machine_verbs() {
    for args in [
        &["guard", "--provider", "claude-code"][..],
        &["git-credential", "get"],
        &["upgrade"],
        &["session", "env"],
        &["graph", "mcp"],
    ] {
        assert!(is_machine_verb(&command(args)), "{args:?}");
    }
}

#[test]
fn ordinary_verbs_are_not_machine_verbs() {
    for args in [
        &["status"][..],
        &["doctor"],
        &["sync"],
        &["feature", "list"],
    ] {
        assert!(!is_machine_verb(&command(args)), "{args:?}");
    }
}

#[test]
fn json_marks_the_run_as_machine_output() {
    assert!(notice_context(true, &command(&["status"])).machine_output);
    assert!(!notice_context(false, &command(&["status"])).machine_output);
}

#[test]
fn a_test_build_is_never_a_release_build() {
    assert!(!notice_context(false, &command(&["status"])).release_build);
}
