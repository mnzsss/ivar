//! `docs/reference/on-disk-format.md` presents the file versions as a stable
//! contract. Its "Current versions" table must match the constants that own
//! each number, so bumping one without the other fails the build.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use crate::domain::agent::receipt::RECEIPT_VERSION;
use crate::domain::feature::{
    CLEANUP_RECORD_SCHEMA_VERSION, FEATURE_CURRENT_VERSION, RUN_CURRENT_VERSION,
};
use crate::domain::session::CURRENT_VERSION as SESSION_CURRENT_VERSION;
use crate::store::manifest::MANIFEST_CURRENT_VERSION;

const DOC: &str = include_str!("../../docs/reference/on-disk-format.md");

/// The rows of the "Current versions" table, keyed by the backticked file
/// column and valued by the version column.
fn documented_versions() -> BTreeMap<String, u32> {
    let section = DOC
        .split("## Current versions")
        .nth(1)
        .expect("doc has a `## Current versions` section");
    let section = section.split("\n#").next().unwrap();

    let mut rows = BTreeMap::new();
    for line in section.lines().filter(|line| line.starts_with('|')) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let [file, version] = cells.as_slice() else {
            panic!("table row must have two columns: {line}");
        };
        let Ok(version) = version.parse::<u32>() else {
            continue; // header and separator rows
        };
        let file = file.trim_matches('`').to_owned();
        assert!(
            rows.insert(file.clone(), version).is_none(),
            "duplicate row for `{file}`"
        );
    }
    rows
}

#[test]
fn current_versions_table_matches_the_version_constants() {
    let expected: BTreeMap<String, u32> = [
        ("ivar.json", MANIFEST_CURRENT_VERSION),
        (".ivar/features/<feature>/feature.json", FEATURE_CURRENT_VERSION),
        (".ivar/features/<feature>/execution/run.json", RUN_CURRENT_VERSION),
        ("<view dir>/state.json", SESSION_CURRENT_VERSION),
        (".ivar/agents/runs/<agent>/<run-id>.json", RECEIPT_VERSION),
        ("docs/updates/<NNN>-<feature>.cleanup.json", CLEANUP_RECORD_SCHEMA_VERSION),
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
fn the_documented_layout_does_not_list_a_hall_level_state_file() {
    assert!(
        !DOC.contains(".ivar/state.json"),
        "no current hall has `.ivar/state.json`; session state is `state.json` in each view dir"
    );
}
