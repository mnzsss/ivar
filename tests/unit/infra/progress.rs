#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use indicatif::ProgressDrawTarget;

fn hidden() -> Stderr {
    Stderr::with_target(ProgressDrawTarget::hidden)
}

#[test]
fn nothing_is_drawn_until_the_first_step() {
    // `reporter(true)` builds this for every human tty run, including verbs
    // that never report progress and hook verbs: they must get no spinner.
    let reporter = hidden();
    assert!(!reporter.is_started());
    reporter.clear();
    assert!(
        !reporter.is_started(),
        "clear on an idle reporter starts nothing"
    );
}

#[test]
fn a_step_starts_the_spinner_with_that_message_and_the_next_replaces_it() {
    let reporter = hidden();
    reporter.step("[1/2] acme: fetching");
    assert!(reporter.is_started());
    assert_eq!(reporter.message().as_deref(), Some("[1/2] acme: fetching"));
    reporter.step("[2/2] web: fetching");
    assert_eq!(reporter.message().as_deref(), Some("[2/2] web: fetching"));
}

#[test]
fn clear_removes_the_spinner_idempotently_and_a_later_step_starts_a_new_one() {
    let reporter = hidden();
    reporter.step("indexing");
    reporter.clear();
    assert!(!reporter.is_started());
    reporter.clear();
    assert!(!reporter.is_started());
    reporter.step("again");
    assert_eq!(reporter.message().as_deref(), Some("again"));
}

#[test]
fn control_characters_become_spaces_so_the_spinner_stays_on_one_line() {
    assert_eq!(one_line("a\nb\tc\rd"), "a b c d");
    let reporter = hidden();
    reporter.step("two\nlines");
    assert_eq!(reporter.message().as_deref(), Some("two lines"));
}
#[test]
fn a_reporter_nobody_wants_is_silent_even_with_a_terminal() {
    // `--json` is the caller saying no. The tty half cannot override it.
    let reporter = reporter(false);
    reporter.step("acme: fetching");
    reporter.clear();
}

#[test]
fn a_reporter_is_silent_when_stderr_is_not_a_terminal() {
    // The test process's stderr is captured, not a tty, so this exercises the
    // real decision rather than a stubbed one.
    if !term::is_tty(Stream::Stderr) {
        let reporter = reporter(true);
        reporter.step("acme: fetching");
        reporter.clear();
    }
}
