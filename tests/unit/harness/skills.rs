#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use std::collections::BTreeSet;

#[test]
fn catalog_is_complete_unique_and_current() {
    let skills = catalog();
    assert_eq!(skills.len(), 1);

    let ids = skills.iter().map(|skill| skill.id).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), skills.len());

    for skill in skills {
        assert_eq!(skill.skill_dir_name(), format!("ivar-{}", skill.id));
        assert_eq!(
            skill.skill_file_rel_path(),
            format!("ivar-{}/SKILL.md", skill.id)
        );
        assert!(skill.content.starts_with("---\n"));
        assert!(skill.content.contains("name:"));
        assert!(skill.content.contains("description:"));
    }
}

#[test]
fn materialise_writes_shipped_skill_and_removes_when_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();

    let changes = materialise(dir).expect("materialise succeeds");
    assert!(!changes.is_empty());
    assert!(dir.join("ivar-execute/SKILL.md").is_file());

    let removed = remove(dir).expect("remove succeeds");
    assert!(!removed.is_empty());
    assert!(!dir.join("ivar-execute/SKILL.md").exists());
}

#[test]
fn materialise_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();

    let first = materialise(dir).expect("first materialise succeeds");
    assert_eq!(first[0].change, Change::Created);

    let second = materialise(dir).expect("second materialise succeeds");
    assert_eq!(second[0].change, Change::Unchanged);
}

#[test]
fn inspect_reports_correct_integrity() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();

    // Before materialise, enabled inspection reports Missing
    let before = inspect(dir, true).expect("inspect succeeds");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].integrity, Integrity::Missing);

    // After materialise, enabled inspection reports Current
    materialise(dir).expect("materialise succeeds");
    let after = inspect(dir, true).expect("inspect succeeds");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].integrity, Integrity::Current);

    // Disabled inspection reports Stale
    let disabled = inspect(dir, false).expect("inspect succeeds");
    assert_eq!(disabled.len(), 1);
    assert_eq!(disabled[0].integrity, Integrity::Stale);
}

#[test]
fn ivar_execute_skill_content_satisfies_all_invariants() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "execute")
        .expect("skill exists");
    let content = skill.content;

    // Invariants
    assert!(content.contains("subagent"));
    assert!(content.contains("Lightweight validation"));
    assert!(content.contains("Deferred validation failures"));
    assert!(content.contains("Standards review"));
    assert!(content.contains("Spec review"));
    assert!(content.contains("Ask"));
    assert!(content.contains("Draft"));
}

#[test]
fn ivar_execute_skill_documents_affected_graph_guidance_and_fallback() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "execute")
        .expect("skill exists");
    let content = skill.content;

    assert!(content.contains("ivar graph affected"));
    assert!(content.contains("advisory"));
    assert!(content.contains("fallback") || content.contains("fall back"));
}
