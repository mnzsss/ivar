#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::provider::Provider;
use crate::domain::skill::SkillRoot;
use crate::domain::skill_sync::{InstallationEntry, ProviderEntry, State, TargetId};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};
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

fn set_providers(root: &camino::Utf8Path, available: Vec<Provider>) {
    let layout = Layout::at(root.to_path_buf());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest
        .with_providers(Providers::new(available, Provider::ClaudeCode))
        .unwrap();
    Manifest::write(&layout, &updated).unwrap();
}

#[test]
fn sync_materialises_only_the_hall_providers_targets() {
    let (_guard, root) = seeded_hall(); // providers: [claude-code]
    write_skill(&root, "audit", "Review a codebase");

    let report = sync(&Ctx::new(root.clone())).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.steps, 1);
    assert!(fs::exists(&root.join(".claude/skills/audit")).unwrap());
    assert!(!fs::exists(&root.join(".opencode")).unwrap());
    assert!(!fs::exists(&root.join(".omp/skills")).unwrap());
}

#[test]
fn a_deleted_config_dir_of_a_removed_provider_is_not_recreated() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "audit", "Review a codebase");
    set_providers(&root, vec![Provider::ClaudeCode, Provider::OpenCode]);
    sync(&Ctx::new(root.clone())).unwrap();
    assert!(fs::exists(&root.join(".opencode/skills/audit")).unwrap());

    set_providers(&root, vec![Provider::ClaudeCode]);
    fs::remove_path(&root.join(".opencode")).unwrap();
    sync(&Ctx::new(root.clone())).unwrap();

    assert!(!fs::exists(&root.join(".opencode")).unwrap());
}

#[test]
fn sync_removes_targets_of_a_provider_dropped_from_the_manifest() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "audit", "Review a codebase");
    set_providers(&root, vec![Provider::ClaudeCode, Provider::OpenCode]);
    sync(&Ctx::new(root.clone())).unwrap();
    // A file ivar never recorded, and a hall-owned sibling of skills/.
    fs::ensure_dir(&root.join(".opencode/skills/mine")).unwrap();
    fs::write_text(&root.join(".opencode/package.json"), "{}\n").unwrap();

    set_providers(&root, vec![Provider::ClaudeCode]); // hand edit, no `provider remove`
    let report = sync(&Ctx::new(root.clone())).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.steps, 1);
    assert!(!fs::exists(&root.join(".opencode/skills/audit")).unwrap());
    assert!(fs::exists(&root.join(".opencode/skills/mine")).unwrap());
    assert!(fs::exists(&root.join(".opencode/package.json")).unwrap());
    assert!(fs::exists(&root.join(".claude/skills/audit")).unwrap());

    let state = skill::read(&root, SkillRoot::Hall).unwrap().unwrap();
    let providers = &state.installations.get("audit").unwrap().providers;
    assert!(providers.contains_key(&TargetId::Claude));
    assert!(!providers.contains_key(&TargetId::OpenCode));
}

#[test]
fn an_emptied_skills_dir_of_a_removed_provider_is_removed_but_not_its_config_dir() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "audit", "Review a codebase");
    set_providers(&root, vec![Provider::ClaudeCode, Provider::OpenCode]);
    sync(&Ctx::new(root.clone())).unwrap();
    fs::write_text(&root.join(".opencode/package.json"), "{}\n").unwrap();

    set_providers(&root, vec![Provider::ClaudeCode]);
    sync(&Ctx::new(root.clone())).unwrap();

    assert!(!fs::exists(&root.join(".opencode/skills")).unwrap());
    assert!(fs::exists(&root.join(".opencode/package.json")).unwrap());
}

#[test]
fn a_recorded_copy_with_a_hall_relative_path_is_removed_under_the_hall_root() {
    // State as 0.16 wrote it: hall-relative `target_path`, copy mode,
    // provider no longer available. The test process cwd is not the hall.
    let (_guard, root) = seeded_hall();
    let copy = root.join(".omp/skills/ghost");
    fs::ensure_dir(&copy).unwrap();
    fs::write_text(&copy.join("SKILL.md"), "copied\n").unwrap();
    let mut installations = std::collections::HashMap::new();
    installations.insert(
        "ghost".to_owned(),
        InstallationEntry {
            source_path: root.join(".ivar/skills-local/ghost"),
            source_hash: "mtime:1".to_owned(),
            installed_at: "1Z".to_owned(),
            commit_sha: None,
            providers: [(
                TargetId::Omp,
                ProviderEntry {
                    target_path: camino::Utf8PathBuf::from(".omp/skills/ghost"),
                    rendered_hash: "mtime:1".to_owned(),
                    linked_at: "1Z".to_owned(),
                    mode: Some(crate::domain::skill::RenderMode::Copy),
                },
            )]
            .into_iter()
            .collect(),
        },
    );
    skill::write(&root, SkillRoot::Local, &State { installations }).unwrap();

    let report = sync(&Ctx::new(root.clone())).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.steps, 1);
    assert!(!fs::exists(&copy).unwrap());
    assert!(skill::read(&root, SkillRoot::Local).unwrap().is_none());
}

#[test]
fn cleanup_is_idempotent() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "audit", "Review a codebase");
    set_providers(&root, vec![Provider::ClaudeCode, Provider::OpenCode]);
    sync(&Ctx::new(root.clone())).unwrap();
    set_providers(&root, vec![Provider::ClaudeCode]);
    sync(&Ctx::new(root.clone())).unwrap();

    let again = sync(&Ctx::new(root.clone())).unwrap();

    assert!(again.is_clean());
    assert_eq!(again.value.steps, 0);
}

#[test]
fn running_sync_twice_changes_nothing() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "refactor", "Restructure code");
    let ctx = Ctx::new(root.clone());

    // First run: creates targets.
    let r1 = sync(&ctx).unwrap();
    assert_eq!(r1.value.steps, 1); // claude-code is the only provider

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
    assert_eq!(r1.value.steps, 3); // 3 skills x 1 target

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
    let state = skill::read(&root, crate::domain::skill::SkillRoot::Hall)
        .unwrap()
        .unwrap();
    assert!(state.installations.contains_key("persist"));
}
