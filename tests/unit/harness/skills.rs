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
        assert!(skill.skill_md().starts_with("---\n"));
        assert!(skill.skill_md().contains("name:"));
        assert!(skill.skill_md().contains("description:"));
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
    assert_eq!(before.len(), catalog().len());
    assert!(before.iter().all(|i| i.integrity == Integrity::Missing));

    // After materialise, enabled inspection reports Current
    materialise(dir).expect("materialise succeeds");
    let after = inspect(dir, true).expect("inspect succeeds");
    assert_eq!(after.len(), catalog().len());
    assert!(after.iter().all(|i| i.integrity == Integrity::Current));

    // Disabled inspection reports Stale
    let disabled = inspect(dir, false).expect("inspect succeeds");
    assert_eq!(disabled.len(), catalog().len());
    assert!(disabled.iter().all(|i| i.integrity == Integrity::Stale));
}

#[test]
fn ivar_execute_skill_content_satisfies_all_invariants() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "execute")
        .expect("skill exists");
    let content = skill.skill_md();

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
    let content = skill.skill_md();

    assert!(content.contains("ivar graph affected"));
    assert!(content.contains("advisory"));
    assert!(content.contains("fallback") || content.contains("fall back"));
}

#[test]
fn materialise_writes_every_declared_file() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();

    materialise(dir).expect("materialise succeeds");

    for skill in catalog() {
        assert!(skill.files.iter().any(|file| file.path == "SKILL.md"));
        for file in skill.files {
            let path = dir.join(skill.skill_dir_name()).join(file.path);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                file.content,
                "{path}"
            );
        }
    }
}

#[test]
fn materialise_prunes_undeclared_files_inside_a_shipped_skill() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    materialise(dir).expect("first materialise succeeds");

    let stray = dir.join("ivar-execute/references/retired.md");
    std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
    std::fs::write(&stray, "retired").unwrap();

    let changes = materialise(dir).expect("second materialise succeeds");

    assert!(!stray.exists());
    let execute = changes
        .iter()
        .find(|change| change.id == "execute")
        .unwrap();
    assert_eq!(execute.change, Change::Updated);
}

#[test]
fn inspect_reports_modified_for_a_missing_declared_file_or_an_undeclared_one() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let integrity = |dir: &camino::Utf8Path| {
        inspect(dir, true)
            .unwrap()
            .into_iter()
            .find(|inspection| inspection.id == "execute")
            .unwrap()
            .integrity
    };

    materialise(dir).unwrap();
    std::fs::remove_file(dir.join("ivar-execute/SKILL.md")).unwrap();
    assert_eq!(integrity(dir), Integrity::Modified);

    materialise(dir).unwrap();
    let stray = dir.join("ivar-execute/references/retired.md");
    std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
    std::fs::write(&stray, "retired").unwrap();
    assert_eq!(integrity(dir), Integrity::Modified);

    materialise(dir).unwrap();
    assert_eq!(integrity(dir), Integrity::Current);
}

#[test]
fn ivar_execute_ships_a_subagent_template_with_absolute_context() {
    let skill = catalog().iter().find(|s| s.id == "execute").unwrap();
    let template = skill
        .files
        .iter()
        .find(|file| file.path == "references/subagent.md")
        .expect("subagent template shipped")
        .content;

    for field in [
        "Working directory",
        "Repository",
        "Worktree root",
        "Task packet",
        "Plan",
        "Requirements",
        "Allowed files",
        "Allowed commands",
        "absolute",
    ] {
        assert!(template.contains(field), "template is missing `{field}`");
    }
    assert!(skill.skill_md().contains("references/subagent.md"));
}
