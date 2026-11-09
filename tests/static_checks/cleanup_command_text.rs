const CLEANUP_COMMAND: &str = include_str!("../../src/harness/commands/feature-cleanup.md");

#[test]
fn cleanup_command_shows_the_fingerprint_as_the_preview_prints_it() {
    assert!(
        !CLEANUP_COMMAND.contains("sha256:"),
        "the preview prints bare hex"
    );
    assert!(
        !CLEANUP_COMMAND.contains("Recompute the preview SHA-256"),
        "ivar computes the fingerprint, never the agent"
    );
    assert!(CLEANUP_COMMAND.contains("never edit `fingerprint` by hand"));
    assert!(CLEANUP_COMMAND.contains("Blockers are not part of it"));
}
