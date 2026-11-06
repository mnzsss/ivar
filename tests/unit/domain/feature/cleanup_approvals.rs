#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;

fn record_with_paths(paths: &[&str]) -> CleanupRecord {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "feature": "feature-cleanup",
        "branch": "feature-cleanup",
        "fingerprint": "sha256:0123",
        "approvals": {
            "delivery": { "approved": true, "at": "2026-08-28T12:00:00Z" },
            "documentation": { "decision": "written", "paths": paths, "reason": null, "at": "2026-08-28T12:05:00Z" },
            "teardown": { "approved": true, "at": "2026-08-28T12:10:00Z" }
        },
        "outcome": null
    }))
    .unwrap()
}

#[test]
fn documentation_paths_must_sit_under_memory_pages_or_repo_relations() {
    for ok in [
        "docs/memory/product/003-x.md",
        "docs/memory/update/007-y.md",
        "docs/repo-relations/001-a-b.md",
    ] {
        assert!(record_with_paths(&[ok]).validate().is_ok(), "{ok}");
    }
    for bad in [
        "docs/product/003-x.md",
        "docs/updates/007-y.md",
        "docs/wayfinder/x.md",
        "HALL.md",
    ] {
        let error = record_with_paths(&[bad]).validate().unwrap_err();
        assert!(error.contains("docs/memory"), "{bad}: {error}");
    }
}

fn record(version: u32, documentation: Option<serde_json::Value>) -> CleanupRecord {
    let mut approvals = serde_json::json!({
        "delivery": { "approved": true, "at": "2026-11-06T01:53:28Z" },
        "teardown": { "approved": true, "at": "2026-11-06T01:54:09Z" }
    });
    if let Some(documentation) = documentation {
        approvals["documentation"] = documentation;
    }
    serde_json::from_value(serde_json::json!({
        "schema_version": version,
        "feature": "memory-retrieval",
        "branch": "memory-retrieval",
        "fingerprint": "247bba99",
        "approvals": approvals,
        "outcome": null
    }))
    .unwrap()
}

fn written() -> serde_json::Value {
    serde_json::json!({
        "at": "2026-11-06T01:53:28Z",
        "decision": "written",
        "paths": ["docs/memory/product/035-memory-pages.md", "docs/memory/update/150-memory-retrieval.md"],
        "reason": null
    })
}

#[test]
fn schema_one_still_requires_and_checks_its_documentation() {
    assert!(record(1, Some(written())).validate().is_ok());
    let missing = record(1, None).validate().unwrap_err();
    assert!(missing.contains("documentation"), "{missing}");
}

#[test]
fn schema_two_has_no_documentation_and_serializes_without_it() {
    let two = record(2, None);
    assert!(two.validate().is_ok());
    assert!(
        serde_json::to_value(&two).unwrap()["approvals"]
            .get("documentation")
            .is_none()
    );

    let with_docs = record(2, Some(written())).validate().unwrap_err();
    assert!(with_docs.contains("schema 2"), "{with_docs}");
}

#[test]
fn the_current_schema_is_two() {
    assert_eq!(CLEANUP_RECORD_SCHEMA_VERSION, 2);
}
