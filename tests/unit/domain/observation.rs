use super::*;

#[test]
fn importance_is_fixed_per_event_as_the_capture_ticket_states() {
    assert_eq!(Importance::session_started().get(), 1);
    assert_eq!(Importance::session_ended().get(), 1);
    assert_eq!(Importance::session_prompted(Origin::User).get(), 5);
    assert_eq!(Importance::session_prompted(Origin::Harness).get(), 1);
    assert_eq!(Importance::session_tool(ToolEffect::Failed).get(), 6);
    assert_eq!(Importance::session_tool(ToolEffect::Writes).get(), 4);
    assert_eq!(Importance::session_tool(ToolEffect::Reads).get(), 2);
}

#[test]
fn source_events_round_trip_through_their_names() {
    for event in [
        SourceEvent::SessionStarted,
        SourceEvent::SessionPrompted,
        SourceEvent::SessionTool,
        SourceEvent::SessionEnded,
    ] {
        assert_eq!(SourceEvent::parse(event.as_str()), Some(event));
        assert!(event.as_str().starts_with("session."));
    }
    assert_eq!(
        SourceEvent::parse("delivery.pr"),
        Some(SourceEvent::DeliveryPr)
    );
    assert_eq!(SourceEvent::parse("delivery.nope"), None);
}

#[test]
fn event_id_is_stable_and_distinguishes_time_and_process() {
    let at = "2026-11-04T10:00:00.000000001Z";
    let a = event_id("native-1", SourceEvent::SessionTool, at, 100);
    let b = event_id("native-1", SourceEvent::SessionTool, at, 100);
    let later = event_id(
        "native-1",
        SourceEvent::SessionTool,
        "2026-11-04T10:00:00.000000002Z",
        100,
    );
    let other_process = event_id("native-1", SourceEvent::SessionTool, at, 101);
    assert_eq!(a, b);
    assert_ne!(a, later);
    assert_ne!(
        a, other_process,
        "two hooks in the same nanosecond are two events"
    );
}

#[test]
fn debug_of_an_observation_never_prints_the_body() {
    let observation = Observation {
        event_id: "e1".to_owned(),
        source_event: "session.tool".to_owned(),
        occurred_at: "2026-11-04T10:00:00.000000000Z".to_owned(),
        session_id: String::new(),
        native_session_id: "n1".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0.19.0".to_owned(),
        feature: String::new(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_ended(),
        attempt_id: String::new(),
        body: "secret-body-text".to_owned(),
    };

    let printed = format!("{observation:?}");

    assert!(!printed.contains("secret-body-text"), "{printed}");
    assert!(printed.contains("e1"), "{printed}");
}

#[test]
fn memory_events_round_trip_and_carry_their_fixed_importance() {
    let expected = [
        (SourceEvent::MemoryPageWritten, "memory.page_written", 4),
        (SourceEvent::MemoryReviewed, "memory.reviewed", 5),
        (SourceEvent::MemoryReviewSkipped, "memory.review_skipped", 7),
    ];
    let importance = [
        Importance::memory_page_written(),
        Importance::memory_reviewed(),
        Importance::memory_review_skipped(),
    ];
    for ((event, name, level), importance) in expected.into_iter().zip(importance) {
        assert_eq!(event.as_str(), name);
        assert_eq!(SourceEvent::parse(name), Some(event));
        assert_eq!(importance.get(), level, "{name}");
    }
}

#[test]
fn all_lists_every_event_once() {
    let names: std::collections::BTreeSet<&str> = SourceEvent::ALL
        .iter()
        .map(|event| event.as_str())
        .collect();
    assert_eq!(names.len(), SourceEvent::ALL.len());
    assert_eq!(SourceEvent::ALL.len(), 18);
    assert!(names.contains("memory.review_skipped"));
}
