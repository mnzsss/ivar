#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn an_event_goes_to_the_conversation_whose_window_holds_it() {
    let windows = [(0, 100), (5000, 9000)];
    assert_eq!(attribute(&windows, 50), Some(0));
    assert_eq!(attribute(&windows, 100 + IDLE_SECS), Some(0));
    assert_eq!(attribute(&windows, 6000), Some(1));
}

#[test]
fn overlapping_windows_pick_the_latest_started() {
    assert_eq!(attribute(&[(0, 1000), (500, 600)], 550), Some(1));
}

#[test]
fn without_a_window_the_most_recent_earlier_conversation_wins() {
    let windows = [(0, 100), (200, 300)];
    assert_eq!(attribute(&windows, 300 + IDLE_SECS + 1), Some(1));
    assert_eq!(attribute(&windows, -5), None);
}
