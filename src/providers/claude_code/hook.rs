use crate::providers::ManagedArtifact;

/// Claude Code manages its hooks and environment directly inside
/// `.claude/settings.json`, which `settings.rs` reconciles.
/// It has no standalone file artifacts.
pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![]
}
