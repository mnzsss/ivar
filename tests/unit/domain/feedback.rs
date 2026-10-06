//! Unit tests for `crate::domain::feedback`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use super::*;
use std::collections::BTreeMap;

#[test]
fn slug_formats_alphanumeric_runs_joined_by_hyphens() {
    assert_eq!(slug("Hello World!"), "hello-world");
    assert_eq!(slug("  --- multiple   spaces --- "), "multiple-spaces");
    assert_eq!(slug(""), "entry");
    assert_eq!(slug("!!!"), "entry");

    let long_title = "a".repeat(100);
    assert_eq!(slug(&long_title).len(), 48);
}

#[test]
fn entry_id_formats_seq_and_slug() {
    assert_eq!(entry_id(1, "Fix crash on init"), "001-fix-crash-on-init");
    assert_eq!(entry_id(42, "Add feature"), "042-add-feature");
}

#[test]
fn next_seq_calculates_max_prefix_plus_one() {
    let ids = [
        "001-first",
        "002-second",
        "010-tenth",
        "invalid",
        "005-five",
    ];
    assert_eq!(next_seq(ids), 11);

    let empty: [&str; 0] = [];
    assert_eq!(next_seq(empty), 1);
}

#[test]
fn redaction_replaces_hall_home_user_host() {
    let r = Redactions {
        hall: Some("/home/alice/projects/valhalla".to_string()),
        home: Some("/home/alice".to_string()),
        user: Some("alice".to_string()),
        host: Some("workstation-01".to_string()),
    };

    let input = "Crash in /home/alice/projects/valhalla/src/main.rs by alice on workstation-01; see /home/alice/.config";
    let output = redact(input, &r);
    assert_eq!(
        output,
        "Crash in <hall>/src/main.rs by <user> on <host>; see ~/.config"
    );
}

#[test]
fn issue_body_and_title_formatting() {
    let entry = FeedbackEntry {
        id: "001-fix-crash".to_string(),
        frontmatter: Frontmatter {
            title: "Fix crash on startup".to_string(),
            kind: FeedbackKind::Bug,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:00:00Z".to_string(),
            ivar_version: "0.14.0".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            provider: Some("claude-code".to_string()),
            session: Some("sess-123".to_string()),
            feature: Some("checkout".to_string()),
            published_url: None,
            extra: BTreeMap::new(),
        },
        body: "Detailed error description here".to_string(),
    };

    assert_eq!(issue_title(&entry), "Fix crash on startup");
    let body = issue_body(&entry);
    assert!(body.contains("### Environment"));
    assert!(body.contains("ivar version: 0.14.0"));
    assert!(body.contains("OS/Arch: linux/x86_64"));
    assert!(body.contains("Provider: claude-code"));
    assert!(body.contains("Detailed error description here"));
    // Session id must not be included
    assert!(!body.contains("sess-123"));
}
