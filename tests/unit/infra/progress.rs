#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_message_that_fits_is_unchanged() {
    assert_eq!(fit("acme: fetching", 80), "acme: fetching");
}

#[test]
fn a_message_exactly_the_width_is_not_truncated() {
    assert_eq!(fit("abcde", 5), "abcde");
}

#[test]
fn a_longer_message_is_cut_and_ellipsised_to_the_width() {
    let line = fit("abcdefghij", 5);
    assert_eq!(line, "abcd…");
    assert_eq!(line.chars().count(), 5);
}

#[test]
fn a_zero_width_produces_nothing() {
    assert_eq!(fit("anything", 0), "");
}

#[test]
fn a_width_of_one_produces_only_the_ellipsis() {
    assert_eq!(fit("anything", 1), "…");
}

#[test]
fn control_characters_become_spaces_so_the_redraw_stays_on_one_line() {
    // A newline would move the cursor off the row the next `\r` returns to,
    // and the erase would then blank the wrong line.
    assert_eq!(fit("a\nb\tc\rd", 80), "a b c d");
}

#[test]
fn flattening_happens_before_truncation_so_the_width_still_holds() {
    let line = fit("aaaa\nbbbb", 5);
    assert_eq!(line.chars().count(), 5);
    assert!(!line.contains('\n'));
}

#[test]
fn silent_reports_nothing_and_never_panics() {
    let silent = Silent;
    silent.step("acme: fetching");
    silent.clear();
    silent.clear();
}

#[test]
fn clearing_a_stderr_reporter_that_never_stepped_writes_nothing() {
    let reporter = Stderr::new();
    reporter.clear();
    assert_eq!(*reporter.live(), 0);
}

#[test]
fn a_step_remembers_the_line_length_and_a_clear_forgets_it() {
    let reporter = Stderr::new();
    reporter.step("acme");
    assert_eq!(*reporter.live(), 4);
    reporter.clear();
    assert_eq!(*reporter.live(), 0);
    // Idempotent: a second clear has nothing to erase.
    reporter.clear();
    assert_eq!(*reporter.live(), 0);
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
