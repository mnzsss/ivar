#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Keeps the "Current versions" table in `docs/reference/on-disk-format.md`
//! equal to the version constants the code stamps on disk.

use std::collections::BTreeMap;

use crate::domain::agent::receipt::RECEIPT_VERSION;
use crate::domain::feature::{CLEANUP_RECORD_SCHEMA_VERSION, Feature, RUN_CURRENT_VERSION};
use crate::domain::name::{BranchName, FeatureName};
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::store::{feature, manifest, session};

const DOC: &str = include_str!("../../../docs/reference/on-disk-format.md");

/// The rows of the "Current versions" table, keyed by the file cell's text
/// with its backticks removed.
fn documented_versions() -> BTreeMap<String, u32> {
    let section = DOC
        .split("\n## Current versions\n")
        .nth(1)
        .expect("doc has a `## Current versions` section");
    let section = section.split("\n#").next().unwrap();

    section
        .lines()
        .filter(|line| line.starts_with('|'))
        .skip(2) // header and separator rows
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 2, "malformed table row: {line}");
            let version = cells[1]
                .parse()
                .unwrap_or_else(|_| panic!("version is not a number: {line}"));
            (cells[0].replace('`', ""), version)
        })
        .collect()
}

#[test]
fn current_versions_table_matches_the_code() {
    let feature_name = FeatureName::new("checkout").unwrap();
    let branch = BranchName::new("checkout").unwrap();
    let domain_feature = Feature::new(feature_name, branch).version();
    let domain_session = SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00Z").version();

    // The domain type and the store each own a copy of the number; both
    // must agree with each other as well as with the doc.
    assert_eq!(domain_feature, feature::CURRENT_VERSION);
    assert_eq!(domain_session, session::CURRENT_VERSION);

    let expected: BTreeMap<String, u32> = [
        ("ivar.json", manifest::CURRENT_VERSION),
        (
            ".ivar/features/<feature>/feature.json",
            feature::CURRENT_VERSION,
        ),
        (
            ".ivar/features/<feature>/execution/run.json",
            RUN_CURRENT_VERSION,
        ),
        ("<view dir>/state.json", session::CURRENT_VERSION),
        (".ivar/agents/runs/<agent>/<run-id>.json", RECEIPT_VERSION),
        ("cleanup record", CLEANUP_RECORD_SCHEMA_VERSION),
    ]
    .into_iter()
    .map(|(file, version)| (file.to_owned(), version))
    .collect();

    assert_eq!(
        documented_versions(),
        expected,
        "docs/reference/on-disk-format.md `Current versions` table drifted from the code"
    );
}

#[test]
fn layout_lists_no_hall_level_state_file() {
    assert!(
        !DOC.contains(".ivar/state.json"),
        "no hall-level `.ivar/state.json` exists; session state lives in each View Dir"
    );
}
