#![allow(clippy::unwrap_used)]

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
