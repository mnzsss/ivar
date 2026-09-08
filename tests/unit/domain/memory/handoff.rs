use crate::domain::memory::handoff::HandoffPayload;
use crate::domain::name::SessionId;

#[test]
fn test_handoff_payload_sanitizes_secrets() {
    let raw_secret = "ghp_123456789012345678901234567890123456";
    let summary = format!("Implemented auth using {raw_secret}");
    let tasks = vec![format!("Revoke token {raw_secret}")];
    let decisions = vec![format!("Do not hardcode {raw_secret}")];
    let paths = vec!["src/main.rs".to_string()];

    let handoff = HandoffPayload::new(
        "handoff-1",
        SessionId::new("11111111-1111-1111-1111-111111111111").unwrap(),
        summary,
        tasks,
        decisions,
        paths,
    );

    assert_eq!(handoff.id, "handoff-1");
    assert!(!handoff.summary.contains("ghp_"));
    assert!(handoff.summary.contains("[REDACTED]"));
    assert!(!handoff.open_tasks[0].contains("ghp_"));
    assert!(handoff.open_tasks[0].contains("[REDACTED]"));
    assert!(!handoff.decisions[0].contains("ghp_"));
    assert!(handoff.decisions[0].contains("[REDACTED]"));
}
