#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::observation::SourceEvent;

#[test]
fn every_session_event_is_swept_and_nothing_else() {
    let session_events: Vec<&str> = SourceEvent::ALL
        .iter()
        .map(|event| event.as_str())
        .filter(|name| name.starts_with("session."))
        .collect();

    assert_eq!(SESSION_EVENTS.to_vec(), session_events);
}

#[test]
fn the_cutoff_is_the_given_days_before_now() {
    assert_eq!(
        cutoff_before(1_800_000_000, 30),
        rfc3339_from_epoch_secs(1_800_000_000.0 - 30.0 * 86_400.0)
    );
}
