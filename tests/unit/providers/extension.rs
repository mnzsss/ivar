// tests/unit/providers/extension.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;
use crate::domain::provider::Provider;
use crate::providers::{self, omp};

#[test]
fn omp_extension_artifact_declared_at_exact_path_and_dependency_free() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let ext_path = Utf8Path::new(".omp/extensions/ivar.js");
    let artifact = artifacts
        .iter()
        .find(|a| a.relative_path == ext_path)
        .unwrap_or_else(|| panic!("expected artifact at {ext_path}, found: {artifacts:?}"));

    assert_eq!(artifact.contents, omp::extension::OMP_EXTENSION);

    // Banner comment distinct from pre-tool guard hook
    assert!(
        artifact
            .contents
            .contains("// ivar autocomplete extension for OMP")
    );
    assert!(!artifact.contents.contains("// ivar pre-tool guard hook for OMP"));

    // OMP loads extension modules as ESM default-exported functions
    assert!(artifact.contents.contains("export default function"));
    assert!(artifact.contents.contains("addAutocompleteProvider"));

    // Plain ESM and node builtins only; no CommonJS require
    assert!(!artifact.contents.contains("require("));
    assert!(artifact.contents.contains("node:child_process"));

    // R-COMP-BOUND: Early exit when session is already bound to a feature
    assert!(artifact.contents.contains("process.env.IVAR_FEATURE"));

    // R-COMP-COMMANDS: Exactly the 7 commands taking an existing feature
    let completed_commands = [
        "/ivar-connect",
        "/ivar-promote",
        "/ivar-deliver",
        "/ivar-feature-status",
        "/ivar-feature-cleanup",
        "/ivar-plan",
        "/ivar-review",
    ];
    for cmd in completed_commands {
        assert!(
            artifact.contents.contains(cmd),
            "extension must match command prefix: {cmd}"
        );
    }

    // R-COMP-EXCLUDE: The 7 commands taking no existing feature must not be listed as triggers
    let excluded_commands = [
        "/ivar-feature-create",
        "/ivar-discovery",
        "/ivar-execute",
        "/ivar-relations",
        "/ivar-repo-setup",
        "/ivar-repo-list",
        "/ivar-sync",
    ];
    for cmd in excluded_commands {
        assert!(
            !artifact.contents.contains(cmd),
            "extension must NOT complete excluded command: {cmd}"
        );
    }

    // R-COMP-SOURCE: Candidate fetching via ivar CLI
    assert!(artifact.contents.contains("execFileSync"));
    assert!(artifact.contents.contains("\"ivar\""));
    assert!(artifact.contents.contains("\"feature\""));
    assert!(artifact.contents.contains("\"list\""));
    assert!(artifact.contents.contains("\"--json\""));

    // R-COMP-DELEGATE: Complete method delegation surface
    // Required methods
    assert!(artifact.contents.contains("getSuggestions"));
    assert!(artifact.contents.contains("applyCompletion"));
    // Optional methods forwarded conditionally
    assert!(artifact.contents.contains("getInlineHint"));
    assert!(artifact.contents.contains("trySyncSlashCompletion"));
    assert!(artifact.contents.contains("trySyncInlineReplace"));
    assert!(artifact.contents.contains("getForceFileSuggestions"));
    assert!(artifact.contents.contains("shouldTriggerFileCompletion"));
}
