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
fn list_reports_no_skills_in_a_fresh_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.skills.is_empty());
}

#[test]
fn list_reports_skills_with_their_descriptions() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "refactor", "Safely restructure code");
    write_skill(&root, "audit", "Review a codebase for issues");
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert_eq!(report.value.skills.len(), 2);
    let audit = report.value.skills.first().unwrap();
    assert_eq!(audit.id.as_str(), "audit");
    assert_eq!(audit.description, "Review a codebase for issues");
    assert_eq!(report.value.skills.get(1).unwrap().id.as_str(), "refactor");
}

#[test]
fn a_malformed_skill_lists_with_an_empty_description() {
    let (_guard, root) = seeded_hall();
    let dir = Layout::at(root.clone()).hall_skills().join("broken");
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("SKILL.md"), "no frontmatter here\n").unwrap();
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert_eq!(report.value.skills.len(), 1);
    let broken = report.value.skills.first().unwrap();
    assert_eq!(broken.id.as_str(), "broken");
    assert!(broken.description.is_empty());
}

#[test]
fn the_human_surface_lists_skills_with_descriptions() {
    let outcome = ListOutcome {
        root: Utf8PathBuf::from("/hall"),
        skills: vec![SkillSummary {
            id: RepoName::new("audit").unwrap(),
            description: "Review a codebase".to_owned(),
            root: SkillRoot::Hall,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Skills in /hall:\n  audit  Review a codebase\n"
    );
}
