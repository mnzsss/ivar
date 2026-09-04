#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::skill::sync as skill_sync;
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
fn doctor_reports_no_problems_in_a_fresh_synced_hall() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "healthy");
    let ctx = Ctx::new(root.clone());

    // Sync first to create targets.
    let _ = skill_sync::sync(&ctx).unwrap();

    let outcome = doctor(&ctx).unwrap();
    assert_eq!(outcome.value.count, 0);
    assert!(outcome.value.problems.is_empty());
}

#[test]
fn doctor_detects_a_missing_target() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "broken");
    let ctx = Ctx::new(root.clone());

    // Sync once to create targets.
    let _ = skill_sync::sync(&ctx).unwrap();

    // Remove the Claude target.
    let claude_target = root.join(".claude").join("skills").join("broken");
    fs::remove_path(&claude_target).unwrap();

    let outcome = doctor(&ctx).unwrap();
    assert!(outcome.value.count > 0);
    assert!(
        outcome
            .value
            .problems
            .iter()
            .any(|p| p.code == "skill.target_missing")
    );
}

#[test]
fn doctor_detects_a_missing_omp_target() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "broken_omp");
    let ctx = Ctx::new(root.clone());

    // Sync once to create targets.
    let _ = skill_sync::sync(&ctx).unwrap();

    // Remove the OMP target.
    let omp_target = root.join(".omp").join("skills").join("broken_omp");
    fs::remove_path(&omp_target).unwrap();

    let outcome = doctor(&ctx).unwrap();
    assert!(outcome.value.count > 0);
    assert!(
        outcome
            .value
            .problems
            .iter()
            .any(|p| p.code == "skill.target_missing" && p.subject.contains("omp"))
    );
}

#[test]
fn doctor_handles_an_empty_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let outcome = doctor(&ctx).unwrap();
    assert_eq!(outcome.value.count, 0);
}

#[test]
fn the_human_surface_reports_problems_with_fixes() {
    let outcome = DoctorOutcome {
        root: Utf8PathBuf::from("/hall"),
        count: 1,
        problems: vec![Problem {
            code: "skill.target_missing",
            subject: "audit@claude".to_owned(),
            what: "materialised target for `audit` at `/target` is missing".to_owned(),
            fix_action: FixAction::safe("skill.sync", "Run `ivy skill sync` to repair.")
                .command("ivy skill sync"),
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("1 problem found:"));
    assert!(text.contains("skill.target_missing"));
    assert!(text.contains("fix: Run `ivy skill sync` to repair."));
}

#[test]
fn the_human_surface_reports_clean_state() {
    let outcome = DoctorOutcome {
        root: Utf8PathBuf::from("/hall"),
        count: 0,
        problems: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), "No problems found.\n");
}
