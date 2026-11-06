#![allow(clippy::unwrap_used)]

use super::*;
use crate::domain::name::FeatureName;

fn feature(name: &str) -> FeatureName {
    FeatureName::new(name).unwrap()
}

fn viewer(session: Option<&str>, provider: &str, feature_name: Option<&str>) -> Viewer {
    Viewer {
        session_key: session.map(str::to_owned),
        provider: provider.to_owned(),
        feature: feature_name.map(feature),
    }
}

#[test]
fn a_session_sees_its_feature_queue_and_the_hall_queue_only() {
    let me = viewer(Some("s1"), "claude-code", Some("login"));

    assert!(me.sees(&Queue::Hall, None));
    assert!(me.sees(&Queue::Feature(feature("login")), None));
    assert!(!me.sees(&Queue::Feature(feature("billing")), None));
}

#[test]
fn outside_a_feature_only_the_hall_queue_is_visible() {
    let discovery = viewer(Some("s2"), "omp", None);

    assert!(discovery.sees(&Queue::Hall, None));
    assert!(!discovery.sees(&Queue::Feature(feature("login")), None));
}

#[test]
fn an_address_hides_the_item_from_everyone_but_its_recipient() {
    let claude = viewer(Some("s1"), "claude-code", None);
    let outside = viewer(None, "claude-code", None);

    assert!(claude.sees(&Queue::Hall, Some(&Address::Provider("claude-code".into()))));
    assert!(!claude.sees(&Queue::Hall, Some(&Address::Provider("omp".into()))));
    assert!(claude.sees(&Queue::Hall, Some(&Address::Session("s1".into()))));
    assert!(!claude.sees(&Queue::Hall, Some(&Address::Session("s9".into()))));
    assert!(outside.sees(&Queue::Hall, Some(&Address::Provider("claude-code".into()))));
    assert!(!outside.sees(&Queue::Hall, Some(&Address::Session("s1".into()))));
}

#[test]
fn queue_and_address_round_trip_through_their_columns() {
    assert_eq!(Queue::Hall.column(), "");
    assert_eq!(Queue::from_column(""), Some(Queue::Hall));
    assert_eq!(
        Queue::from_column("login"),
        Some(Queue::Feature(feature("login")))
    );
    assert_eq!(Queue::from_column("Not A Name"), None);

    let address = Address::Session("s1".into());
    assert_eq!((address.kind(), address.value()), ("session", "s1"));
    assert_eq!(Address::from_columns("session", "s1"), Some(address));
    assert_eq!(Address::from_columns("", ""), None);
    assert_eq!(Address::from_columns("team", "x"), None);
}

#[test]
fn states_round_trip_and_the_limits_match_the_brief() {
    for state in [
        HandoffState::Open,
        HandoffState::Accepted,
        HandoffState::Cancelled,
        HandoffState::Expired,
    ] {
        assert_eq!(HandoffState::parse(state.as_str()), Some(state));
    }
    for state in [
        MessageState::Pending,
        MessageState::Popped,
        MessageState::Cancelled,
        MessageState::Expired,
    ] {
        assert_eq!(MessageState::parse(state.as_str()), Some(state));
    }
    assert_eq!(HandoffState::Open.as_str(), "open");
    assert_eq!(MessageState::Pending.as_str(), "pending");
    assert_eq!(MAX_TEXT_BYTES, 16 * 1024);
    assert_eq!(MAX_LIST_ITEMS, 50);
    assert_eq!(DEFAULT_TTL_DAYS, 7);
    assert_eq!(TERMINAL_RETENTION_DAYS, 30);
}
