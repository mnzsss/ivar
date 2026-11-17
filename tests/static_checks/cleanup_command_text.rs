//! Text checks over the shipped `feature-cleanup` command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

#[test]
fn cleanup_command_shows_the_fingerprint_as_the_preview_prints_it() {
    let cleanup = shipped("commands/feature-cleanup.md");
    assert!(!cleanup.contains("sha256:"), "the preview prints bare hex");
    assert!(
        !cleanup.contains("Recompute the preview SHA-256"),
        "ivar computes the fingerprint, never the agent"
    );
    assert!(cleanup.contains("never edit `fingerprint` by hand"));
    assert!(cleanup.contains("Blockers are not part of it"));
}

#[test]
fn cleanup_command_ends_with_a_non_blocking_automation_checkpoint() {
    let cleanup = shipped("commands/feature-cleanup.md");
    let step = cleanup
        .find("### Step 6: Automation Checkpoint")
        .expect("the checkpoint step");
    for gate in ["### Step 2: Delivery Gate", "### Step 4: Teardown Gate"] {
        assert!(cleanup.find(gate).expect(gate) < step, "after {gate}");
    }
    let body = &cleanup[step..];
    assert!(body.contains("ivar automation scan --feature <feature> --json"));
    assert!(body.contains("`ivar-automate` skill"));
    assert!(body.contains("never blocks cleanup"));
    assert!(body.contains("never produces a warning"));
    assert!(body.contains("Retirable automations never appear here"));
}

const FINGERPRINT_PLACEHOLDER: &str = "<hex from --preview --json>";

/// The first fenced `json` block after `### JSON Schema`.
fn schema_example(doc: &str) -> &str {
    let heading = doc
        .find("### JSON Schema")
        .expect("the JSON Schema heading");
    let after = &doc[heading..];
    let open = after.find("```json\n").expect("a json block") + "```json\n".len();
    let body = &after[open..];
    &body[..body.find("```").expect("the json block closes")]
}

#[test]
fn cleanup_command_record_example_parses_as_a_cleanup_record() {
    let cleanup = shipped("commands/feature-cleanup.md");
    let example = schema_example(&cleanup);
    assert!(
        example.contains(FINGERPRINT_PLACEHOLDER),
        "the example marks where the preview hex goes"
    );

    // The preview prints a SHA-256 as bare lowercase hex.
    let hex = "a".repeat(64);
    let json = example.replace(FINGERPRINT_PLACEHOLDER, &hex);

    let record: ivar::domain::feature::CleanupRecord = serde_json::from_str(&json)
        .unwrap_or_else(|err| panic!("the documented record must deserialize: {err}"));
    record
        .validate()
        .unwrap_or_else(|err| panic!("the documented record must validate: {err}"));
    assert_eq!(record.fingerprint, hex);
    assert!(
        record.approvals.delivery.approved && record.approvals.teardown.approved,
        "the example is an applicable record"
    );
}
