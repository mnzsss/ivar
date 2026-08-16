#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::error::Status;
use crate::infra::frontmatter;
use crate::test_support::seeded_hall;

#[test]
fn create_writes_sk_md_with_name_and_description_frontmatter() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            id: "refactor".to_owned(),
            description: "Safely restructure code".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    let source = fs::read_text(&report.value.skill_file).unwrap().unwrap();
    let meta: serde_json::Value = frontmatter::parse::<serde_json::Value>(&source).unwrap();
    assert_eq!(meta.get("name").unwrap(), "refactor");
    assert_eq!(meta.get("description").unwrap(), "Safely restructure code");
}

#[test]
fn create_is_rejected_for_a_duplicate_skill() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(
        &ctx,
        CreateInput {
            id: "refactor".to_owned(),
            description: "first".to_owned(),
        },
    )
    .unwrap();

    let failure = create(
        &ctx,
        CreateInput {
            id: "refactor".to_owned(),
            description: "second".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "skill.already_exists");
}

#[test]
fn create_rejects_an_invalid_id() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = create(
        &ctx,
        CreateInput {
            id: "../etc".to_owned(),
            description: "x".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "name.not_a_segment");
}

#[test]
fn the_human_surface_names_the_skill_file() {
    let outcome = CreateOutcome {
        root: Utf8PathBuf::from("/hall"),
        id: RepoName::new("refactor").unwrap(),
        skill_file: Utf8PathBuf::from("/hall/.ivar/skills/refactor/SKILL.md"),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created skill `refactor` at /hall/.ivar/skills/refactor/SKILL.md\n"
    );
}
