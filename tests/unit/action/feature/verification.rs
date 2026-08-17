//! Unit tests for `crate::action::feature::verification` — the ordered
//! executable checks runner and its deterministic fingerprint.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::{canonical_temp_dir, utf8_temp_dir};

/// Canonicalised, unlike its siblings here, because this is the one test that
/// compares a path against what the shell reports: on macOS `TempDir` hands
/// back `/var/...` whose real name is `/private/var/...`, and `pwd` prints the
/// resolved one.
#[test]
fn commands_run_in_order_in_the_exact_worktree() {
    let (_guard, worktree) = canonical_temp_dir();
    let commands = vec![
        "printf one >> order".to_owned(),
        "printf two >> order".to_owned(),
        "pwd > here".to_owned(),
    ];

    let report = run(&commands, &worktree).unwrap();

    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.command.as_str())
            .collect::<Vec<_>>(),
        ["printf one >> order", "printf two >> order", "pwd > here"]
    );
    assert!(report.results.iter().all(|result| result.success));
    let order = std::fs::read_to_string(worktree.join("order")).unwrap();
    assert_eq!(order, "onetwo");
    let here = std::fs::read_to_string(worktree.join("here")).unwrap();
    assert_eq!(here.trim(), worktree.as_str());
}

#[test]
fn execution_stops_at_the_first_failure() {
    let (_guard, worktree) = utf8_temp_dir();
    let commands = vec![
        "printf first >> order".to_owned(),
        "exit 7".to_owned(),
        "printf third >> order".to_owned(),
    ];

    let report = run(&commands, &worktree).unwrap();

    assert_eq!(report.results.len(), 2, "the third command must not run");
    assert!(report.results[0].success);
    assert!(!report.results[1].success);
    assert_eq!(report.results[1].exit_code, Some(7));
    let order = std::fs::read_to_string(worktree.join("order")).unwrap();
    assert_eq!(order, "first");
    assert!(
        !order.contains("third"),
        "the command after the failure must not run"
    );
}

#[test]
fn an_empty_command_list_is_a_success() {
    let (_guard, worktree) = utf8_temp_dir();

    let report = run(&[], &worktree).unwrap();

    assert!(report.results.is_empty());
    assert!(!report.command_fingerprint.is_empty());
}

#[test]
fn a_spawn_failure_is_a_recorded_failure_that_stops_the_run() {
    let (_guard, worktree) = utf8_temp_dir();
    // `bash -lc` of a command whose program does not exist still spawns bash;
    // the failure shows up as a nonzero exit with a diagnostic naming it.
    let commands = vec!["definitely-not-a-real-program-xyz".to_owned()];

    let report = run(&commands, &worktree).unwrap();

    assert_eq!(report.results.len(), 1);
    assert!(!report.results[0].success);
    assert_eq!(report.results[0].exit_code, Some(127));
    assert!(
        report.results[0]
            .diagnostic
            .contains("definitely-not-a-real-program-xyz"),
        "the diagnostic must name the command: {}",
        report.results[0].diagnostic
    );
}

#[test]
fn a_failure_captures_the_diagnostic() {
    let (_guard, worktree) = utf8_temp_dir();
    let commands = vec!["echo boom >&2; exit 3".to_owned()];

    let report = run(&commands, &worktree).unwrap();

    assert_eq!(report.results.len(), 1);
    assert!(!report.results[0].success);
    assert_eq!(report.results[0].exit_code, Some(3));
    assert!(report.results[0].diagnostic.contains("boom"));
}

#[test]
fn the_fingerprint_is_deterministic_and_sensitive_to_the_commands() {
    let commands = vec!["cargo fmt --check".to_owned(), "cargo test".to_owned()];
    let other = vec!["cargo test".to_owned(), "cargo fmt --check".to_owned()];

    let a = fingerprint(&commands).unwrap();
    assert_eq!(a, fingerprint(&commands).unwrap());
    assert_ne!(a, fingerprint(&other).unwrap());
}
