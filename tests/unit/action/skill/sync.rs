#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn write_skill(root: &camino::Utf8Path, id: &str, description: &str) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("SKILL.md"),
        &format!("---\nname: {id}\ndescription: {description}\n---\n\nBody.\n"),
    )
    .unwrap();
}

#[test]
fn sync_materializes_to_claude_and_opencode_targets() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "audit", "Review a codebase");
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx).unwrap();
    assert!(report.is_clean());

    // Check that targets were created.
    let claude_target = root.join(".claude").join("skills").join("audit");
    let opencode_target = root.join(".opencode").join("skills").join("audit");
    assert!(fs::exists(&claude_target).unwrap());
    assert!(fs::exists(&opencode_target).unwrap());
}

#[test]
fn running_sync_twice_changes_nothing() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "refactor", "Restructure code");
    let ctx = Ctx::new(root.clone());

    // First run: creates targets.
    let r1 = sync(&ctx).unwrap();
    assert_eq!(r1.value.steps, 2); // Claude + OpenCode

    // Second run: no steps because everything matches.
    let r2 = sync(&ctx).unwrap();
    assert!(r2.is_clean());
    assert_eq!(r2.value.steps, 0);
}

#[test]
fn a_removed_target_is_repaired_on_next_sync() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "lint", "Check code style");
    let ctx = Ctx::new(root.clone());

    // Sync once to create targets.
    sync(&ctx).unwrap();

    // Remove the Claude target manually.
    let claude_target = root.join(".claude").join("skills").join("lint");
    fs::remove_path(&claude_target).unwrap();
    assert!(!fs::exists(&claude_target).unwrap());

    // Sync again: repairs the missing target.
    let report = sync(&ctx).unwrap();
    assert!(report.is_clean());

    assert!(fs::exists(&claude_target).unwrap());
}

#[test]
fn sync_is_idempotent_with_multiple_skills() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "alpha", "First skill");
    write_skill(&root, "beta", "Second skill");
    write_skill(&root, "gamma", "Third skill");
    let ctx = Ctx::new(root.clone());

    let r1 = sync(&ctx).unwrap();
    assert_eq!(r1.value.steps, 6); // 3 skills x 2 targets

    let r2 = sync(&ctx).unwrap();
    assert_eq!(r2.value.steps, 0);
}

#[test]
fn sync_handles_an_empty_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx).unwrap();
    assert!(report.is_clean());
    assert_eq!(report.value.steps, 0);
}

#[test]
fn sync_updates_the_state_lockfile() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "persist", "Should survive");
    let ctx = Ctx::new(root.clone());

    sync(&ctx).unwrap();

    // State file should exist and contain the skill.
    let state = skill::read(&root, crate::domain::skill::SkillRoot::Hall).unwrap().unwrap();
    assert!(state.installations.contains_key("persist"));
}
