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
