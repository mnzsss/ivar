#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn explicit_override_wins_over_everything() {
    assert!(decide_colour(Some(true), Some(""), Some("0"), false));
    assert!(!decide_colour(Some(false), None, Some("1"), true));
}

#[test]
fn no_color_set_to_anything_disables_colour() {
    assert!(!decide_colour(None, Some(""), None, true));
    assert!(!decide_colour(None, Some("0"), None, true));
    assert!(!decide_colour(None, Some("whatever"), None, true));
}

#[test]
fn no_color_beats_force_color() {
    assert!(!decide_colour(None, Some(""), Some("1"), true));
}

#[test]
fn force_color_set_to_zero_disables_colour() {
    assert!(!decide_colour(None, None, Some("0"), true));
}

#[test]
fn force_color_set_to_anything_else_enables_colour() {
    assert!(decide_colour(None, None, Some("1"), false));
    assert!(decide_colour(None, None, Some(""), false));
    assert!(decide_colour(None, None, Some("true"), false));
}

#[test]
fn falls_back_to_tty_detection() {
    assert!(decide_colour(None, None, None, true));
    assert!(!decide_colour(None, None, None, false));
}

#[test]
fn width_never_panics_and_has_a_positive_fallback() {
    assert!(width() > 0);
}

#[test]
fn a_real_answer_is_taken_as_the_width() {
    assert_eq!(decide_width(Some(120)), 120);
    assert_eq!(decide_width(Some(1)), 1);
}

/// A pty with no window size set answers `Ok((0, 0))` instead of failing.
/// Zero columns is a missing answer, not a narrow terminal — a caller laying
/// out against it produces nothing at all.
#[test]
fn zero_columns_falls_back_like_a_failed_query() {
    assert_eq!(decide_width(Some(0)), DEFAULT_WIDTH);
    assert_eq!(decide_width(None), DEFAULT_WIDTH);
}

#[test]
fn colour_does_not_panic_and_is_stable_across_calls() {
    let first = colour(None);
    let second = colour(None);
    assert_eq!(first, second);
}

#[test]
fn colour_for_stdout_agrees_with_colour() {
    assert_eq!(colour_for(Stream::Stdout, None), colour(None));
}

#[test]
fn colour_for_is_stable_across_calls_for_either_stream() {
    assert_eq!(
        colour_for(Stream::Stderr, None),
        colour_for(Stream::Stderr, None)
    );
}

#[test]
fn an_explicit_override_reaches_both_streams() {
    // The pure rule is what the cached wrappers delegate to; asserting it
    // per stream here documents that the flag is global while only the tty
    // fallback is per-stream.
    for tty in [true, false] {
        assert!(decide_colour(Some(true), None, None, tty));
        assert!(!decide_colour(Some(false), None, None, tty));
    }
}

#[test]
fn a_redirected_stream_gets_no_colour_even_when_the_other_is_a_tty() {
    // stderr redirected to a file (not a tty) with no env overrides: the
    // fallback must answer false regardless of stdout being a terminal.
    assert!(!decide_colour(None, None, None, false));
    assert!(decide_colour(None, None, None, true));
}

#[test]
fn is_tty_does_not_panic_for_either_stream() {
    let _ = is_tty(Stream::Stdout);
    let _ = is_tty(Stream::Stderr);
}
