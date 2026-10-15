#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::session::SessionState;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const FIRST: &str = "00000000-0000-4000-8000-000000000001";
const SECOND: &str = "00000000-0000-4000-8000-000000000002";
const EARLIEST: &str = "00000000-0000-4000-8000-000000000004";
const STATELESS: &str = "00000000-0000-4000-8000-000000000003";

fn hall() -> (tempfile::TempDir, Ctx, Layout) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root);
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    let layout = discover_hall(&ctx).unwrap();
    (guard, ctx, layout)
}

fn discovery_session(layout: &Layout, id: &str, started_at: &str) -> Utf8PathBuf {
    let view_dir = layout.discovery_session(&SessionId::new(id).unwrap());
    fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::OpenCode, started_at)
        .write(&view_dir)
        .unwrap();
    view_dir
}

fn feature_session(layout: &Layout, feature: &str, id: &str, started_at: &str) -> Utf8PathBuf {
    let feature = FeatureName::new(feature).unwrap();
    let view_dir = layout.feature_session(&feature, &SessionId::new(id).unwrap());
    fs::ensure_dir(&view_dir).unwrap();
    let mut state = SessionState::new(Provider::ClaudeCode, started_at);
    state.bind(feature, started_at);
    state.write(&view_dir).unwrap();
    view_dir
}

#[test]
fn an_empty_hall_lists_no_sessions_cleanly() {
    let (_guard, ctx, _layout) = hall();

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.sessions.is_empty());
    let mut human = Vec::new();
    report.value.write_human(&mut human).unwrap();
    assert_eq!(String::from_utf8(human).unwrap(), "No live sessions.\n");
}

#[test]
fn lists_discovery_and_feature_sessions_with_their_record() {
    let (_guard, ctx, layout) = hall();
    let discovery_dir = discovery_session(&layout, FIRST, "2026-01-01T00:00:00.000000000Z");
    let feature_dir = feature_session(
        &layout,
        "checkout",
        SECOND,
        "2026-01-02T00:00:00.000000000Z",
    );

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(discovery_dir.is_absolute());
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(
        json["sessions"],
        serde_json::json!([
            {
                "id": FIRST,
                "kind": "discovery",
                "feature": null,
                "provider": "opencode",
                "started_at": "2026-01-01T00:00:00.000000000Z",
                "feature_bound_at": null,
                "view_dir": discovery_dir.as_str(),
            },
            {
                "id": SECOND,
                "kind": "feature",
                "feature": "checkout",
                "provider": "claude-code",
                "started_at": "2026-01-02T00:00:00.000000000Z",
                "feature_bound_at": "2026-01-02T00:00:00.000000000Z",
                "view_dir": feature_dir.as_str(),
            },
        ])
    );
}

#[test]
fn orders_by_started_at_then_id() {
    let (_guard, ctx, layout) = hall();
    feature_session(
        &layout,
        "checkout",
        SECOND,
        "2026-01-02T00:00:00.000000000Z",
    );
    discovery_session(&layout, FIRST, "2026-01-02T00:00:00.000000000Z");
    feature_session(
        &layout,
        "billing",
        EARLIEST,
        "2026-01-01T00:00:00.000000000Z",
    );

    let sessions = list(&ctx).unwrap().value.sessions;

    let ids: Vec<&str> = sessions.iter().map(|session| session.id.as_str()).collect();
    assert_eq!(ids, [EARLIEST, FIRST, SECOND]);
}

#[test]
fn a_session_without_a_record_is_listed_with_nulls_and_a_warning() {
    let (_guard, ctx, layout) = hall();
    fs::ensure_dir(&layout.discovery_session(&SessionId::new(STATELESS).unwrap())).unwrap();

    let report = list(&ctx).unwrap();

    assert!(!report.is_clean());
    assert_eq!(report.warnings[0].code, "session.state_unreadable");
    assert_eq!(report.warnings[0].subject, STATELESS);
    let session = &report.value.sessions[0];
    assert_eq!(session.kind, SessionKind::Discovery);
    assert_eq!(session.provider, None);
    assert_eq!(session.started_at, None);
}

#[test]
fn listing_writes_nothing() {
    let (_guard, ctx, layout) = hall();
    let view_dir = feature_session(
        &layout,
        "checkout",
        SECOND,
        "2026-01-02T00:00:00.000000000Z",
    );
    let record_before = std::fs::read(view_dir.join("state.json")).unwrap();
    let entries_before = fs::read_dir(&view_dir).unwrap();

    list(&ctx).unwrap();

    assert_eq!(
        std::fs::read(view_dir.join("state.json")).unwrap(),
        record_before
    );
    assert_eq!(fs::read_dir(&view_dir).unwrap(), entries_before);
}
