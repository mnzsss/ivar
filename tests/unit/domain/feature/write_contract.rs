//! Unit tests for `crate::domain::feature::write_contract` — the
//! glob-matching write contract each workstream must respect.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;

use super::*;

// -- WriteContract ---------------------------------------------------------

#[test]
fn write_contract_allows_exact_path() {
    let contract = WriteContract::new(vec!["src/action/execute/tick.rs".to_owned()]);
    assert!(contract.allows(Utf8Path::new("src/action/execute/tick.rs")));
    assert!(!contract.allows(Utf8Path::new("src/action/execute/approve.rs")));
}

#[test]
fn write_contract_allows_directory_prefix() {
    let contract = WriteContract::new(vec!["src/domain/".to_owned()]);
    assert!(contract.allows(Utf8Path::new("src/domain/feature.rs")));
    assert!(contract.allows(Utf8Path::new("src/domain/name.rs")));
    // The prefix itself is allowed — a directory glob covers the dir too.
    assert!(contract.allows(Utf8Path::new("src/domain")));
    // A sibling with the same textual prefix is not covered.
    assert!(!contract.allows(Utf8Path::new("src/domain_extra/file.rs")));
}

#[test]
fn write_contract_allows_glob() {
    let contract = WriteContract::new(vec!["src/action/skill/*.rs".to_owned()]);
    assert!(contract.allows(Utf8Path::new("src/action/skill/sync.rs")));
    assert!(contract.allows(Utf8Path::new("src/action/skill/doctor.rs")));
    assert!(!contract.allows(Utf8Path::new("src/action/execute/tick.rs")));
}

#[test]
fn write_contract_rejects_dot_dot_escape() {
    let contract = WriteContract::new(vec!["src/".to_owned()]);
    assert!(!contract.allows(Utf8Path::new("../hall.json")));
    assert!(!contract.allows(Utf8Path::new("src/../../outside")));
}

#[test]
fn write_contract_defaults_to_deny() {
    let contract = WriteContract::new(Vec::new());
    assert!(!contract.allows(Utf8Path::new("anything.rs")));
}
