//! Unit tests for `crate::store::manifest::error`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::error::Status;

// -- Error -> Failure: every variant has its own code and fix action -----

#[test]
fn missing_version_failure_names_the_path_and_a_safe_fix() {
    let failure: Failure = Error::MissingVersion {
        path: Utf8PathBuf::from("/hall/ivar.json"),
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.missing_version");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn default_provider_not_available_failure_names_the_offending_value() {
    let failure: Failure = Error::DefaultProviderNotAvailable {
        default: Provider::OpenCode,
        available: vec![Provider::ClaudeCode],
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.default_provider_not_available");
    assert!(failure.what.contains("opencode"));
    assert!(failure.actual.as_deref().unwrap().contains("opencode"));
    assert!(failure.fix_actions[0].what.contains("claude-code"));
}

#[test]
fn no_available_providers_failure_has_a_safe_fix() {
    let failure: Failure = Error::NoAvailableProviders.into();
    assert_eq!(failure.code, "manifest.no_available_providers");
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn duplicate_repo_name_failure_names_the_offending_repo() {
    let failure: Failure = Error::DuplicateRepoName {
        name: RepoName::new("api").unwrap(),
    }
    .into();
    assert_eq!(failure.code, "manifest.duplicate_repo_name");
    assert!(failure.what.contains("api"));
    assert!(failure.fix_actions[0].what.contains("api"));
}

#[test]
fn store_error_delegates_its_failure_conversion() {
    let failure: Failure = Error::Store(versioned::Error::TooNew {
        path: Utf8PathBuf::from("/hall/ivar.json"),
        found: 2,
        highest: 1,
    })
    .into();
    assert_eq!(failure.code, "store.version_too_new");
}
