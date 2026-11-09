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
