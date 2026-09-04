// tests/unit/harness/config/artifact.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::provider::Provider;
use crate::harness::config::Change;
use crate::harness::config::artifact::{reconcile_managed_artifact, remove_managed_artifact};
use crate::infra::fs;
use crate::providers::{self, ManagedArtifact};
use crate::test_support::utf8_temp_dir;

fn artifact_by_path<'a>(artifacts: &'a [ManagedArtifact], path: &str) -> &'a ManagedArtifact {
    artifacts
        .iter()
        .find(|a| a.relative_path == path)
        .unwrap_or_else(|| panic!("artifact with path `{path}` not found in {artifacts:?}"))
}

#[test]
fn omp_hook_materialises_and_is_idempotent() {
    let (_guard, dir) = utf8_temp_dir();
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let artifact = artifact_by_path(&artifacts, ".omp/hooks/pre/ivar.js");
    let dest = dir.join(&artifact.relative_path);

    let first = reconcile_managed_artifact(&dest, artifact.contents).unwrap();
    assert_eq!(first, Change::Created);
    assert!(dest.is_file());
    let on_disk = fs::read_text(&dest).unwrap().unwrap();
    assert_eq!(on_disk, artifact.contents);

    let second = reconcile_managed_artifact(&dest, artifact.contents).unwrap();
    assert_eq!(second, Change::Unchanged);
}

#[test]
fn omp_hook_updates_when_bytes_change() {
    let (_guard, dir) = utf8_temp_dir();
    let dest = dir.join(".omp/hooks/pre/ivar.js");
    fs::ensure_dir(dest.parent().unwrap()).unwrap();
    fs::write_text(&dest, "// old hook content\n").unwrap();

    let artifacts = providers::managed_artifacts(Provider::Omp);
    let artifact = artifact_by_path(&artifacts, ".omp/hooks/pre/ivar.js");

    let change = reconcile_managed_artifact(&dest, artifact.contents).unwrap();
    assert_eq!(change, Change::Updated);
    let on_disk = fs::read_text(&dest).unwrap().unwrap();
    assert_eq!(on_disk, artifact.contents);
}

#[test]
fn omp_hook_removed_when_provider_disabled() {
    let (_guard, dir) = utf8_temp_dir();
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let artifact = artifact_by_path(&artifacts, ".omp/hooks/pre/ivar.js");
    let dest = dir.join(&artifact.relative_path);

    reconcile_managed_artifact(&dest, artifact.contents).unwrap();
    assert!(dest.exists());

    let change = remove_managed_artifact(&dest).unwrap();
    assert_eq!(change, Change::Removed);
    assert!(!dest.exists());

    // Removing again on absent file is idempotent
    let second = remove_managed_artifact(&dest).unwrap();
    assert_eq!(second, Change::Unchanged);
}

#[test]
fn user_owned_sibling_file_in_same_directory_survives() {
    let (_guard, dir) = utf8_temp_dir();
    let hook_path = dir.join(".omp/hooks/pre/ivar.js");
    let user_hook = dir.join(".omp/hooks/pre/custom-audit.js");

    fs::ensure_dir(hook_path.parent().unwrap()).unwrap();
    fs::write_text(&user_hook, "console.log('user custom hook');\n").unwrap();

    let artifacts = providers::managed_artifacts(Provider::Omp);
    let artifact = artifact_by_path(&artifacts, ".omp/hooks/pre/ivar.js");
    reconcile_managed_artifact(&hook_path, artifact.contents).unwrap();

    assert!(hook_path.exists());
    assert!(user_hook.exists());

    remove_managed_artifact(&hook_path).unwrap();
    assert!(!hook_path.exists());
    assert!(
        user_hook.exists(),
        "User file in same directory must never be deleted"
    );
}
