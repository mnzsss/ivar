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
    assert_eq!(skills.len(), 8);

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
fn feedback_skill_content_satisfies_all_invariants() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "feedback")
        .expect("feedback skill is in catalog");

    let content = skill.skill_md();
    assert!(content.contains("name: ivar-feedback"));
    assert!(content.contains("ivar feedback add"));
    assert!(content.contains("## Symptom"));
    assert!(content.contains("## Reproduction"));
    assert!(content.contains("## Cause"));
    assert!(content.contains("## What should change"));
    assert!(content.contains("## Suggested tests"));
}
#[test]
fn feedback_skill_publishes_only_through_an_approved_preview() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "feedback")
        .expect("feedback skill is in catalog");
    let content = skill.skill_md();
    assert!(content.contains("ivar feedback submit <id> --preview"));
    assert!(content.contains("--fingerprint <fingerprint>"));
}

#[test]
fn agent_skill_suggests_agents_only_after_the_user_agrees() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "agent")
        .expect("agent skill is in catalog");
    let content = skill.skill_md();
    assert!(content.contains("name: ivar-agent"));
    assert!(content.contains("## When to suggest an agent"));
    assert!(content.contains("Create nothing\nbefore the answer."));
    assert!(content.contains("## Remove an agent"));
    assert!(content.contains("Agent Task · deferred check"));
    assert!(content.contains("Scheduled agent · declared\nrhythm"));
    assert!(content.contains("ivar automation undo <id>"));
    assert!(!content.contains("third time"));
    assert!(!content.contains("same check"));
    assert!(content.contains("Ask before `ivar agent sync`"));
    assert!(content.contains("Ask before `ivar agent run <name>`"));
    assert!(content.contains("Never ask for a value in\n   the conversation"));
}

fn explore() -> ShippedSkill {
    *catalog()
        .iter()
        .find(|skill| skill.id == "explore")
        .expect("explore is shipped")
}

#[test]
fn explore_skill_routes_on_its_description() {
    let content = explore().skill_md();
    assert!(content.contains("name: ivar-explore"));
    let description = content
        .lines()
        .find_map(|line| line.strip_prefix("description: "))
        .expect("description is a single frontmatter line");
    assert!(
        description.contains("Discovery Session"),
        "was: {description}"
    );
    assert!(description.contains("not scoped yet"), "was: {description}");
    assert!(
        description.contains("Do not use it for a plain question about existing code"),
        "was: {description}"
    );
    assert!(
        description.contains("do not use it inside a Feature Session"),
        "was: {description}"
    );
}

#[test]
fn explore_skill_walks_the_discovery_flow() {
    let content = explore().skill_md();
    for step in [
        "$IVAR_SESSION_ID",
        "ivar session start",
        "ivar discovery create",
        "ivar discovery amend",
        "ivar session convert",
        "irreversible",
        "`ivar-plan`",
        "one question at a time",
        "Never convert automatically",
        "Never write to repositories during discovery",
        "never starts a second one",
        "This session is already a Feature session",
        "Never create or modify session files",
    ] {
        assert!(
            content.to_lowercase().contains(&step.to_lowercase()),
            "missing: {step}"
        );
    }
    assert!(!content.contains("/ivar-discovery"));
    assert!(!content.contains("$ARGUMENTS"));
}

#[test]
fn explore_skill_uses_graph_explore_with_fallback() {
    let content = explore()
        .skill_md()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(content.contains("ivar graph explore"), "was: {content}");
    assert!(content.contains("advisory"), "was: {content}");
    assert!(
        content.contains("fallback") || content.contains("fall back"),
        "was: {content}"
    );
}

#[test]
fn subfeatures_skill_walks_the_parallel_children_loop() {
    let skill = catalog()
        .iter()
        .find(|skill| skill.id == "subfeatures")
        .expect("subfeatures is shipped");
    let body = skill.skill_md();
    for step in [
        "ivar feature create <child> --parent <parent>",
        "ivar feature promote <parent> <repo>",
        "ivar session start <child> --detached",
        "ivar feature status <parent> --recursive",
        "ivar plan approve <child> plan",
        "ivar feature integrate <child>",
        "ivar feature deliver <parent> --preview",
    ] {
        assert!(body.contains(step), "missing: {step}");
    }
    for reference in ["references/child-planner.md", "references/state.md"] {
        assert!(
            skill.files.iter().any(|file| file.path == reference),
            "{reference} is linked from SKILL.md but not shipped"
        );
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
    assert!(content.contains("name: ivar-execute"));
    assert!(content.contains("subagent"));
    assert!(content.contains("Lightweight validation"));
    assert!(content.contains("lightweight validation"));
    assert!(content.contains("Deferred validation failures"));
    assert!(content.contains("Standards review"));
    assert!(content.contains("Spec review"));
    assert!(content.contains("Ask"));
    assert!(content.contains("Draft delivery"));
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

    let deep = dir.join("ivar-plan/extra/deep/x.md");
    std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
    std::fs::write(&deep, "stray").unwrap();

    let changes = materialise(dir).expect("second materialise succeeds");

    assert!(!stray.exists());
    assert!(!deep.exists());
    assert!(!dir.join("ivar-plan/extra").exists());
    assert!(dir.join("ivar-plan").is_dir());
    let execute = changes
        .iter()
        .find(|change| change.id == "execute")
        .unwrap();
    assert_eq!(execute.change, Change::Updated);
}

#[cfg(unix)]
#[test]
fn materialise_unlinks_a_symlink_inside_a_skill_without_touching_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("keep.md");
    std::fs::write(&outside_file, "keep").unwrap();
    materialise(dir).unwrap();

    let link = dir.join("ivar-execute/linked");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();

    materialise(dir).unwrap();

    assert!(outside_file.exists());
    assert!(std::fs::symlink_metadata(&link).is_err());
}

#[test]
fn materialise_restores_a_modified_declared_file() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let plan_change = |dir: &camino::Utf8Path| {
        materialise(dir)
            .unwrap()
            .into_iter()
            .find(|change| change.id == "plan")
            .unwrap()
            .change
    };
    materialise(dir).unwrap();
    let template = dir.join("ivar-plan/references/task-template.md");
    std::fs::write(&template, "edited").unwrap();

    assert_eq!(plan_change(dir), Change::Updated);
    let expected = catalog()
        .iter()
        .find(|s| s.id == "plan")
        .unwrap()
        .files
        .iter()
        .find(|file| file.path == "references/task-template.md")
        .unwrap()
        .content;
    assert_eq!(std::fs::read_to_string(&template).unwrap(), expected);
    assert_eq!(plan_change(dir), Change::Unchanged);
}

#[test]
fn inspect_reports_the_skill_directory_for_a_modified_skill() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    materialise(dir).unwrap();
    std::fs::write(dir.join("ivar-execute/SKILL.md"), "edited").unwrap();

    let execute = inspect(dir, true)
        .unwrap()
        .into_iter()
        .find(|inspection| inspection.id == "execute")
        .unwrap();

    assert_eq!(execute.integrity, Integrity::Modified);
    assert_eq!(execute.path, dir.join("ivar-execute"));
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

fn change_for(dir: &camino::Utf8Path, id: &str) -> Change {
    materialise(dir)
        .unwrap()
        .into_iter()
        .find(|change| change.id == id)
        .unwrap()
        .change
}

fn catalog_content(id: &str, path: &str) -> &'static str {
    catalog()
        .iter()
        .find(|s| s.id == id)
        .unwrap()
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap()
        .content
}

#[test]
fn materialise_reports_updated_when_only_empty_subdirs_are_removed() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    materialise(dir).unwrap();
    let empty = dir.join("ivar-plan/extra/empty");
    std::fs::create_dir_all(&empty).unwrap();

    assert_eq!(change_for(dir, "plan"), Change::Updated);
    assert!(!dir.join("ivar-plan/extra").exists());
    assert_eq!(change_for(dir, "plan"), Change::Unchanged);
}

#[cfg(unix)]
#[test]
fn materialise_replaces_a_symlinked_skill_dir_without_touching_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("SKILL.md");
    std::fs::write(&outside_file, "outside").unwrap();
    let skill_dir = dir.join("ivar-execute");
    std::os::unix::fs::symlink(outside.path(), &skill_dir).unwrap();

    assert_eq!(change_for(dir, "execute"), Change::Created);

    assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "outside");
    assert!(std::fs::symlink_metadata(&skill_dir).unwrap().is_dir());
    assert_eq!(
        std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap(),
        catalog_content("execute", "SKILL.md")
    );
}

#[cfg(unix)]
#[test]
fn materialise_replaces_a_symlinked_declared_parent_without_touching_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    materialise(dir).unwrap();
    let references = dir.join("ivar-execute/references");
    std::fs::remove_dir_all(&references).unwrap();
    std::os::unix::fs::symlink(outside.path(), &references).unwrap();

    materialise(dir).unwrap();

    assert!(!outside.path().join("subagent.md").exists());
    assert!(std::fs::symlink_metadata(&references).unwrap().is_dir());
    assert_eq!(
        std::fs::read_to_string(references.join("subagent.md")).unwrap(),
        catalog_content("execute", "references/subagent.md")
    );
}

#[cfg(unix)]
#[test]
fn inspect_reports_a_symlinked_skill_dir_as_modified() {
    let temp = tempfile::tempdir().unwrap();
    let dir = camino::Utf8Path::from_path(temp.path()).unwrap();
    let intact = tempfile::tempdir().unwrap();
    let intact_dir = camino::Utf8Path::from_path(intact.path()).unwrap();
    materialise(intact_dir).unwrap();
    materialise(dir).unwrap();
    let skill_dir = dir.join("ivar-execute");
    std::fs::remove_dir_all(&skill_dir).unwrap();
    std::os::unix::fs::symlink(intact_dir.join("ivar-execute"), &skill_dir).unwrap();

    let execute = inspect(dir, true)
        .unwrap()
        .into_iter()
        .find(|inspection| inspection.id == "execute")
        .unwrap();

    assert_eq!(execute.integrity, Integrity::Modified);
}

/// The deliver checkpoint sits between preview and apply, and deferring it
/// neither blocks apply nor invalidates the fingerprint.
#[test]
fn deliver_checks_relation_context_between_preview_and_apply() {
    let content = catalog()
        .iter()
        .find(|s| s.id == "deliver")
        .unwrap()
        .skill_md()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    assert!(content.contains("preview"), "was: {content}");
    assert!(content.contains("apply"), "was: {content}");
    assert!(content.contains("HALL.md"), "was: {content}");
    assert!(content.contains("evidence"), "was: {content}");
    assert!(content.contains("/ivar-relations"), "was: {content}");
    assert!(content.contains("fingerprint"), "was: {content}");
    assert!(content.contains("does not block apply"), "was: {content}");
}

#[test]
fn deliver_skill_reviews_memory_before_the_preview_and_never_skips_on_its_own() {
    let skill = catalog()
        .iter()
        .find(|skill| skill.id == "deliver")
        .expect("deliver is shipped");
    let content = skill.skill_md();

    assert!(content.contains("## Review memory before apply"));
    assert!(content.contains("`memory_review`"));
    assert!(content.contains("`confirm: true`"));
    assert!(content.contains("`updated`") && content.contains("`still_valid`"));
    assert!(content.contains("`deliver.memory_not_reviewed`"));
    assert!(content.contains("`deliver.memory_review_outdated`"));
    assert!(content.contains("--skip-memory-review"));
    assert!(content.contains("Never pass it on your own"));
    assert!(content.find("## Review memory before apply") < content.find("## What happens"));
}

#[test]
fn ivar_plan_ships_the_packet_brief_and_cites_it_with_the_lint() {
    let plan = catalog().iter().find(|s| s.id == "plan").unwrap();
    let brief = plan
        .files
        .iter()
        .find(|file| file.path == "references/packet-brief.md")
        .expect("ivar-plan ships references/packet-brief.md");
    assert!(brief.content.contains("git grep -n"));

    let phase3 = plan.skill_md().split("## Phase 3").nth(1).unwrap();
    assert!(phase3.contains("references/packet-brief.md"));
    assert!(phase3.contains("ivar plan lint"));
}
