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
    assert_eq!(SourceEvent::parse("delivery.pr"), None);
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
