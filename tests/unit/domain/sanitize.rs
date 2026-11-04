#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::observation::{Importance, Observation};
use camino::Utf8PathBuf;

fn observation(body: &str) -> Observation {
    Observation {
        event_id: "e1".to_owned(),
        source_event: "session.tool".to_owned(),
        occurred_at: "2026-11-04T10:00:00.000000000Z".to_owned(),
        session_id: String::new(),
        native_session_id: "native-1".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0.19.0".to_owned(),
        feature: String::new(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_ended(),
        attempt_id: String::new(),
        body: body.to_owned(),
    }
}

fn ctx() -> SanitizeContext {
    SanitizeContext::new(
        Utf8PathBuf::from("/hall"),
        vec!["s3cr3t-env-value".to_owned()],
    )
}

const KNOWN_SECRETS: [&str; 6] = [
    "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB",
    "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
    "AKIAIOSFODNN7EXAMPLE",
    "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature",
    "x-api-key: 0123456789abcdef0123456789abcdef",
    "s3cr3t-env-value",
];

#[test]
fn no_known_secret_survives_sanitization() {
    for secret in KNOWN_SECRETS {
        let clean =
            Sanitized::sanitize(observation(&format!("run with {secret} now")), &ctx()).unwrap();
        let body = &clean.as_observation().body;
        assert!(!body.contains(secret), "{secret} leaked into {body}");
        assert!(body.contains("[REDACTED"), "{body}");
    }
}

#[test]
fn paths_outside_the_hall_are_replaced_and_hall_paths_are_kept() {
    let clean = Sanitized::sanitize(
        observation("cat /home/other/.ssh/config and /hall/ivar/src/lib.rs"),
        &ctx(),
    )
    .unwrap();
    let body = &clean.as_observation().body;
    assert!(!body.contains("/home/other"), "{body}");
    assert!(body.contains("/hall/ivar/src/lib.rs"), "{body}");
}

#[test]
fn a_body_over_the_limit_is_truncated_on_a_char_boundary() {
    let mut context = ctx();
    context.body_limit = 8;

    let clean = Sanitized::sanitize(observation("ééééééééééé"), &context).unwrap();

    assert!(clean.as_observation().body.len() <= 8 + "…".len());
}

#[test]
fn sanitizing_twice_changes_nothing() {
    let once = Sanitized::sanitize(
        observation("token ghp_0123456789abcdefghijklmnopqrstuvwxyzAB"),
        &ctx(),
    )
    .unwrap();
    let twice = Sanitized::sanitize(once.as_observation().clone(), &ctx()).unwrap();
    assert_eq!(once.as_observation(), twice.as_observation());
}

#[test]
fn a_redactor_that_leaves_a_secret_behind_fails_closed() {
    let result = sanitize_with(
        observation("ghp_0123456789abcdefghijklmnopqrstuvwxyzAB"),
        &ctx(),
        |text| text.to_owned(),
    );
    assert_eq!(result.unwrap_err(), SanitizeError::ResidualSecret);
}

#[test]
fn debug_of_the_context_never_prints_env_values() {
    let printed = format!("{:?}", ctx());

    assert!(!printed.contains("s3cr3t-env-value"), "{printed}");
    assert!(printed.contains("1 redacted"), "{printed}");
}

#[test]
fn a_huge_body_is_cut_before_redaction_and_still_never_leaks_a_secret() {
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    let filler = "a ".repeat(10 * 1024 * 1024);
    let started = std::time::Instant::now();

    let clean = Sanitized::sanitize(observation(&format!("{filler}{secret}")), &ctx()).unwrap();

    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert!(clean.as_observation().body.len() <= SanitizeContext::DEFAULT_BODY_LIMIT + 3);
    assert!(!clean.as_observation().body.contains("ghp_"));
}

#[test]
fn a_secret_split_by_the_pre_cut_does_not_survive_when_redaction_shrinks_the_text() {
    let mut context = ctx();
    context.body_limit = 64;
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        "A".repeat(449)
    );
    assert_eq!(pem.len(), context.body_limit * PRE_REDACTION_FACTOR - 9);
    let body = format!("{pem} s3cr3t-env-value");

    let clean = Sanitized::sanitize(observation(&body), &context).unwrap();

    assert!(
        !clean.as_observation().body.contains("s3cr3t"),
        "{}",
        clean.as_observation().body
    );
}

#[test]
fn a_body_shortened_by_the_pre_cut_is_marked_even_when_redaction_shrinks_it_under_the_limit() {
    let mut context = ctx();
    context.body_limit = 64;
    let body = format!("sk-{} {}", "a".repeat(400), "z".repeat(400));

    let clean = Sanitized::sanitize(observation(&body), &context).unwrap();

    let stored = &clean.as_observation().body;
    assert!(stored.len() <= context.body_limit, "{stored}");
    assert!(stored.ends_with('…'), "{stored}");
}

#[test]
fn a_body_that_fits_untouched_is_not_marked() {
    let clean = Sanitized::sanitize(observation("short body"), &ctx()).unwrap();

    assert_eq!(clean.as_observation().body, "short body");
}
