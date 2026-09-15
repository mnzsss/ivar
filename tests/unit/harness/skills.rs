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
    assert_eq!(skills.len(), 2);

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

fn plan_text() -> String {
    catalog()
        .iter()
        .find(|skill| skill.id == "plan")
        .expect("plan skill shipped")
        .files
        .iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn ivar_plan_skill_ships_plan_and_task_templates() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "plan")
        .expect("plan skill");
    let file = |path: &str| {
        skill
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .content
    };

    assert!(skill.skill_md().starts_with("---\nname: ivar-plan\n"));
    assert!(skill.skill_md().contains("references/plan-template.md"));
    assert!(skill.skill_md().contains("references/task-template.md"));
    assert!(file("references/plan-template.md").contains("### Wave N"));
    assert!(file("references/plan-template.md").contains("Lightweight validation"));
    assert!(file("references/task-template.md").contains("**Readers:**"));
    assert!(file("references/task-template.md").contains("**Sketch:**"));
}

/// The plan checkpoint sits at the beginning of Analysis: read `HALL.md` and
/// the linked topics of potentially affected Repos, record the context, and
/// never let a deferred review block approval.
#[test]
fn plan_checks_relation_context_at_the_start_of_analysis() {
    let content = plan_text();
    let analysis = content
        .find("## Phase 2: Analysis")
        .expect("plan has an Analysis phase");
    let after = &content[analysis..];
    let lower = after.to_lowercase();

    assert!(lower.contains("read `hall.md`"), "was: {after}");
    assert!(after.contains("linked topics"), "was: {after}");
    assert!(after.contains("`analysis.md`"), "was: {after}");
    assert!(after.contains("evidence"), "was: {after}");
    assert!(after.contains("/ivar-relations"), "was: {after}");
    assert!(after.contains("never blocks"), "was: {after}");
}

#[test]
fn plan_has_three_approval_gates_and_hands_off_to_execute() {
    let content = plan_text();

    assert!(content.contains("approve requirements"), "was: {content}");
    assert!(content.contains("approve analysis"), "was: {content}");
    assert!(content.contains("approve plan"), "was: {content}");
    assert!(content.contains("ivar-execute"), "was: {content}");
    assert!(content.contains("Done"), "was: {content}");
    assert!(content.contains("✅"), "was: {content}");
    assert!(!content.contains("approve graph"), "was: {content}");
}

/// A task packet must declare who reads what it writes, with the grep that
/// found them. Three waves were reverted because a packet edited its declared
/// files and broke a reader outside the list.
#[test]
fn plan_packet_template_requires_declared_readers() {
    let content = plan_text();

    assert!(content.contains("**Readers:**"), "was: {content}");
    assert!(content.contains("git grep -n"), "was: {content}");
    assert!(content.contains("no readers outside"), "was: {content}");
}

/// The Readers grep must cover the whole tree, not a scope someone derives.
/// Two controlled runs found the same failure twice: a reader in `examples/`
/// escaped a hardcoded `src/ tests/`, and a reader in `docs/` escaped a scope
/// derived from the workspace members. Both times every agent reported
/// success. `git grep` with no pathspec ends the derivation: tracked files in,
/// build artifacts out, nothing to get wrong.
#[test]
fn plan_readers_grep_is_scoped_to_the_whole_tree() {
    let content = plan_text();

    assert!(content.contains("git grep -n '<symbol>'"), "was: {content}");
    assert!(content.contains("no pathspec"), "was: {content}");
    assert!(
        content.contains("not only code that compiles"),
        "was: {content}"
    );
}

/// A build check narrower than the readers it must defend passes while a
/// reader outside it breaks. In the controlled run every agent ran the
/// packet's `cargo build`, saw green, and shipped an `examples/` target that
/// failed to compile.
#[test]
fn plan_verification_covers_every_reader_it_declares() {
    let content = plan_text();

    assert!(content.contains("as wide as the readers"), "was: {content}");
}

/// The plan reviewer checks that packets declare their readers. Without this
/// the Readers field is advisory, which is how the parent feature's
/// R-DELEGATE defect was born.
#[test]
fn plan_reviewer_checks_declared_readers() {
    let content = plan_text();

    assert!(content.contains("Blast Radius"), "was: {content}");

    let table = content
        .find("| Category | What to Look For |")
        .expect("plan has a reviewer checklist");
    let after = &content[table..];
    let end = after
        .find("Reviewer output format")
        .expect("checklist ends");
    let checklist = &after[..end];

    assert!(checklist.contains("Blast Radius"), "was: {checklist}");
    assert!(checklist.contains("Readers"), "was: {checklist}");
}

/// The packet template's own Step 1 must show assertions, not describe them.
/// Two shipped plans (`ivar-manifest-schema`, `omp-support`: 20 packets)
/// carried zero code while `:176-180` already forbade it in prose. A rule
/// stated beside a form that contradicts it loses to the form.
#[test]
fn plan_packet_template_requires_literal_test() {
    let content = plan_text();

    assert!(content.contains("literal source"), "was: {content}");
    assert!(
        content.contains("Step 1 carries the test's literal source"),
        "was: {content}"
    );
    // The example inside the template is real code, not a placeholder.
    assert!(content.contains("assert_eq!"), "was: {content}");
}

/// The escape is bounded on both sides: Step 1 may never use it, and a
/// sketch still names its signatures. `omp-support`'s packet 01 produced
/// `Provider::Omp` "with stable id `omp`, config dir `.omp`" — an enum
/// variant whose actual shape no packet ever wrote down, leaving the next
/// packet's `Consumes` citing nothing.
#[test]
fn plan_sketch_escape_is_bounded() {
    let content = plan_text();

    assert!(content.contains("**Sketch:**"), "was: {content}");
    assert!(content.contains("never Step 1"), "was: {content}");
    assert!(
        content.contains("relaxes a body, never an interface"),
        "was: {content}"
    );
    assert!(content.contains("exact signature"), "was: {content}");
}

/// The reviewer checks the code obligation, or it is advisory — which is
/// how the prose at `:176-180` failed. The row must judge the *reason* on a
/// sketch, not merely the marker's presence: an unjudged escape becomes the
/// default.
#[test]
fn plan_reviewer_checks_literal_code() {
    let content = plan_text();

    let table = content
        .find("| Category | What to Look For |")
        .expect("plan has a reviewer checklist");
    let after = &content[table..];
    let end = after
        .find("Reviewer output format")
        .expect("checklist ends");
    let checklist = &after[..end];

    assert!(checklist.contains("Literal Code"), "was: {checklist}");
    assert!(checklist.contains("Step 1"), "was: {checklist}");
    assert!(checklist.contains("**Sketch:**"), "was: {checklist}");
    assert!(checklist.contains("reason"), "was: {checklist}");
}

#[test]
fn plan_uses_graph_evidence_with_fallback_and_no_approval_bypass() {
    let content = plan_text();
    assert!(content.contains("ivar graph explore"), "was: {content}");
    assert!(content.contains("advisory"), "was: {content}");
    assert!(
        content.contains("fallback") || content.contains("fall back"),
        "was: {content}"
    );
    assert!(
        content.contains("never creates, approves, or bypasses"),
        "was: {content}"
    );
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
