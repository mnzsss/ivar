#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

const TARGETS: [&str; 3] = [
    ".claude/skills/audit",
    ".opencode/skills/audit",
    ".omp/skills/audit",
];

fn write_skill(skills_root: Utf8PathBuf, id: &str) {
    let dir = skills_root.join(id);
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("SKILL.md"),
        &format!("---\nname: {id}\ndescription: d\n---\n\nBody.\n"),
    )
    .unwrap();
}

fn input(id: &str) -> ToggleInput {
    ToggleInput {
        skill: id.to_owned(),
    }
}

fn add_provider(ctx: &Ctx, name: &str) {
    crate::action::provider::add::add(
        ctx,
        &crate::action::provider::add::AddInput {
            name: name.to_owned(),
        },
    )
    .unwrap();
}

#[test]
fn disable_removes_the_targets_and_keeps_the_source() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    write_skill(layout.hall_skills(), "audit");
    let ctx = Ctx::new(root.clone());
    add_provider(&ctx, "opencode");
    add_provider(&ctx, "omp");
    crate::action::skill::sync::sync(&ctx).unwrap();
    for target in TARGETS {
        assert!(fs::exists(&root.join(target)).unwrap(), "{target}");
    }

    let outcome = disable(&ctx, &input("audit")).unwrap().value;

    assert!(outcome.changed);
    assert!(!outcome.enabled);
    assert_eq!(outcome.origin, SkillRoot::Hall);
    for target in TARGETS {
        assert!(!fs::exists(&root.join(target)).unwrap(), "{target}");
    }
    assert!(fs::exists(&layout.hall_skills().join("audit/SKILL.md")).unwrap());
    assert!(fs::exists(&root.join(".ivar/skills-local/disabled.json")).unwrap());
    assert!(!fs::exists(&root.join(".ivar/skills/disabled.json")).unwrap());
}

#[test]
fn sync_does_not_bring_a_disabled_skill_back_and_enable_does() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    write_skill(layout.hall_skills_local(), "notes");
    let ctx = Ctx::new(root.clone());
    disable(&ctx, &input("notes")).unwrap();

    crate::action::skill::sync::sync(&ctx).unwrap();
    assert!(!fs::exists(&root.join(".claude/skills/notes")).unwrap());

    let outcome = enable(&ctx, &input("notes")).unwrap().value;

    assert!(outcome.changed);
    assert!(outcome.enabled);
    assert!(fs::exists(&root.join(".claude/skills/notes")).unwrap());
    assert!(!fs::exists(&root.join(".ivar/skills-local/disabled.json")).unwrap());
}

#[test]
fn repeating_the_current_state_changes_nothing() {
    let (_guard, root) = seeded_hall();
    write_skill(Layout::at(root.clone()).hall_skills(), "audit");
    let ctx = Ctx::new(root);

    assert!(!enable(&ctx, &input("audit")).unwrap().value.changed);
    assert!(disable(&ctx, &input("audit")).unwrap().value.changed);
    assert!(!disable(&ctx, &input("audit")).unwrap().value.changed);
}

#[test]
fn an_unknown_or_unsafe_id_is_refused() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    assert_eq!(
        disable(&ctx, &input("ghost")).unwrap_err().code,
        "skill.not_found"
    );
    assert!(disable(&ctx, &input("../skills")).is_err());
}
