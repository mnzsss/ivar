#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::skill::sync;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn write_skill(root: &camino::Utf8Path, id: &str) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("SKILL.md"),
        &format!("---\nname: {id}\ndescription: {id} skill\n---\n\nBody.\n"),
    )
    .unwrap();
}

#[test]
fn remove_deletes_the_skill_directory_and_targets() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "to_remove");
    let ctx = Ctx::new(root.clone());
    let _ = sync::sync(&ctx).unwrap();
    assert!(fs::exists(&root.join(".claude/skills/to_remove")).unwrap());

    let report = remove(
        &ctx,
        &RemoveInput {
            skill: "to_remove".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.id.as_str(), "to_remove");
    assert_eq!(report.value.targets_removed, 1);
    assert!(!fs::exists(&root.join(".ivar/skills/to_remove")).unwrap());
    assert!(!fs::exists(&root.join(".claude/skills/to_remove")).unwrap());
}

#[test]
fn remove_also_tears_down_a_recorded_target_of_a_removed_provider() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "gone");
    let layout = Layout::at(root.clone());
    let manifest = crate::store::manifest::Manifest::read(&layout).unwrap().unwrap();
    let both = manifest
        .with_providers(crate::store::manifest::Providers::new(
            vec![
                crate::domain::provider::Provider::ClaudeCode,
                crate::domain::provider::Provider::Omp,
            ],
            crate::domain::provider::Provider::ClaudeCode,
        ))
        .unwrap();
    crate::store::manifest::Manifest::write(&layout, &both).unwrap();
    let ctx = Ctx::new(root.clone());
    let _ = sync::sync(&ctx).unwrap();
    crate::store::manifest::Manifest::write(&layout, &manifest).unwrap();

    let report = remove(
        &ctx,
        &RemoveInput {
            skill: "gone".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.targets_removed, 2);
    assert!(!fs::exists(&root.join(".omp/skills/gone")).unwrap());
    assert!(!fs::exists(&root.join(".claude/skills/gone")).unwrap());
}

#[test]
fn remove_purges_the_lockfile_entry() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "locked");
    let ctx = Ctx::new(root.clone());

    // Sync to create state.
    let _ = sync::sync(&ctx).unwrap();

    // Verify state exists.
    let state = skill::read(&root, crate::domain::skill::SkillRoot::Hall)
        .unwrap()
        .unwrap();
    assert_eq!(state.installations.len(), 1);

    remove(
        &ctx,
        &RemoveInput {
            skill: "locked".to_owned(),
        },
    )
    .unwrap();

    // Lockfile entry purged.
    let state = skill::read(&root, crate::domain::skill::SkillRoot::Hall).unwrap();
    assert!(
        state.is_none(),
        "lockfile should be absent after removing last skill"
    );
}

#[test]
fn remove_rejects_a_nonexistent_skill() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = remove(
        &ctx,
        &RemoveInput {
            skill: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "skill.not_found");
    assert_eq!(failure.status, crate::error::Status::Blocked);
}

#[test]
fn remove_is_verifiable_by_state_cleanup() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "verify_me");
    let ctx = Ctx::new(root.clone());

    // Sync + remove.
    let _ = sync::sync(&ctx).unwrap();
    remove(
        &ctx,
        &RemoveInput {
            skill: "verify_me".to_owned(),
        },
    )
    .unwrap();

    // No state entries remain.
    let state = skill::read(&root, crate::domain::skill::SkillRoot::Hall).unwrap();
    assert!(state.is_none());

    // No skill directory remains.
    let skills_dir = root.join(".ivar").join("skills");
    let entries = fs::read_dir(&skills_dir).unwrap();
    assert!(entries.is_empty());
}
