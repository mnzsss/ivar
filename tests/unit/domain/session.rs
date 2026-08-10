#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::FeatureName;

fn discovery_state() -> SessionState {
    SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00.000000000Z")
}

#[test]
fn new_creates_a_discovery_record() {
    let state = discovery_state();
    assert!(state.is_discovery());
    assert_eq!(state.provider(), Provider::ClaudeCode);
    assert_eq!(state.started_at(), "2026-01-01T00:00:00.000000000Z");
    assert_eq!(state.feature(), None);
    assert_eq!(state.feature_bound_at(), None);
    assert_eq!(state.version(), 1);
}

#[test]
fn bind_attaches_the_feature_once_and_is_idempotent() {
    let mut state = discovery_state();
    let feature = FeatureName::new("checkout").unwrap();

    state.bind(feature.clone(), "2026-02-02T00:00:00.000000000Z");
    state.bind(feature.clone(), "2026-03-03T00:00:00.000000000Z");

    assert_eq!(state.feature(), Some(&feature));
    assert_eq!(
        state.feature_bound_at(),
        Some("2026-02-02T00:00:00.000000000Z")
    );
    assert!(!state.is_discovery());
}

#[test]
fn session_state_round_trips_through_serde_without_unknown_fields() {
    let mut state = discovery_state();
    state.bind(
        FeatureName::new("checkout").unwrap(),
        "2026-02-02T00:00:00.000000000Z",
    );

    let rendered = serde_json::to_string(&state).unwrap();
    let parsed: SessionState = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, state);
}

#[test]
fn an_unknown_field_in_session_state_is_refused() {
    let raw = r#"{"version":1,"feature":null,"provider":"claude-code","started_at":"2026-01-01T00:00:00.000000000Z","bogus":true}"#;
    assert!(serde_json::from_str::<SessionState>(raw).is_err());
}

#[test]
fn rfc3339_now_is_fixed_width_and_zero_padded() {
    let now = rfc3339_now();
    assert_eq!(now.len(), 30, "was: {now}");
    assert!(now.ends_with('Z'), "was: {now}");
    assert!(now.starts_with("20"), "was: {now}");
}

#[test]
fn civil_from_days_known_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(1), (1970, 1, 2));
    // 2000-03-01: 30 years from 1970 with seven leap days in between, then
    // Jan (31) + Feb (29, 2000 is a leap year).
    assert_eq!(civil_from_days(30 * 365 + 7 + 31 + 29), (2000, 3, 1));
}
