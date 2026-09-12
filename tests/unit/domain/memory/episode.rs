#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::memory::episode::EpisodePayload;
use crate::domain::name::{FeatureName, SessionId};

#[test]
fn test_episode_payload_sanitization_and_markdown() {
    let raw_secret = "ghp_123456789012345678901234567890123456";
    let summary = format!("Finished session with token {raw_secret}");
    let files = vec![format!("src/{raw_secret}.rs")];

    let episode = EpisodePayload::new(
        SessionId::new("11111111-1111-1111-1111-111111111111").unwrap(),
        Some(FeatureName::new("shared-memory").unwrap()),
        "2026-09-08T12:00:00Z",
        "2026-09-08T13:00:00Z",
        summary,
        files,
    );

    assert!(!episode.summary.contains("ghp_"));
    assert!(episode.summary.contains("[REDACTED]"));
    assert!(!episode.files_touched[0].contains("ghp_"));
    assert!(episode.files_touched[0].contains("[REDACTED]"));

    let markdown = episode.render_markdown();
    assert!(markdown.contains("# Session Episode: 11111111-1111-1111-1111-111111111111"));
    assert!(markdown.contains("- **Feature**: `shared-memory`"));
    assert!(markdown.contains("- **Started**: 2026-09-08T12:00:00Z"));
    assert!(markdown.contains("- **Stopped**: 2026-09-08T13:00:00Z"));
    assert!(markdown.contains("## Summary"));
    assert!(markdown.contains("Finished session with token [REDACTED]"));
    assert!(markdown.contains("## Files Touched"));
    assert!(markdown.contains("- `src/[REDACTED].rs`"));
}
