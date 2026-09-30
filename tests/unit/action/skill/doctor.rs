#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

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

fn hall_with_repo_skill(repo_skill: &str) -> (tempfile::TempDir, camino::Utf8PathBuf) {
    use crate::domain::name::{BranchName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers, Repo};

    let (guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            root.join("origin-api").as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let dir = layout
        .repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("main").unwrap(),
        )
        .join(".claude/skills")
        .join(repo_skill);
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("SKILL.md"),
        &format!("---\nname: {repo_skill}\n---\n"),
    )
    .unwrap();
    (guard, root)
}

#[test]
fn repo_skill_problems_reports_a_repo_skill_shadowed_by_a_hall_skill() {
    let (_guard, root) = hall_with_repo_skill("commit");
    write_skill(&root, "commit");
    let layout = Layout::at(root.clone());
    // The hall skill must be visible where harnesses look for it.
    let _ = skill_sync::sync(&Ctx::new(root.clone())).unwrap();

    let problems = repo_skill_problems(&layout, None);

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].code, "skill.repo_prefixed");
    assert_eq!(problems[0].subject, "api/.claude/skills/commit");
    assert!(
        problems[0].what.contains("api--commit"),
        "{}",
        problems[0].what
    );
}

#[test]
fn repo_skill_problems_is_empty_when_names_are_unique() {
    let (_guard, root) = hall_with_repo_skill("react-doctor");

    assert!(repo_skill_problems(&Layout::at(root), None).is_empty());
}
