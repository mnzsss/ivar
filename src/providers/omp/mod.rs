pub mod auth;
pub mod commands;
pub mod extension;
pub mod guard;
pub mod hook;
pub mod launch;
pub mod mcp;
pub mod session;

use crate::providers::ManagedArtifact;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    let mut artifacts = hook::managed_artifacts();
    artifacts.extend(extension::managed_artifacts());
    artifacts
}
