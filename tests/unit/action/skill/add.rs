#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::skill::parse_frontmatter;
use crate::test_support::hall_root;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    (guard, root)
}

#[test]
fn add_writes_sk_md_with_source_frontmatter() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(
        &ctx,
        AddInput {
            repo: "mnzsss/skills".to_owned(),
            path: Some("skills/lint".to_owned()),
            ref_: Some("main".to_owned()),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.id.as_str(), "skills");

    let source = fs::read_text(&report.value.skill_file).unwrap().unwrap();
    let meta = parse_frontmatter(&source).unwrap().unwrap();

    assert_eq!(meta.name, "skills");
    assert_eq!(meta.source.as_ref().unwrap().repo, "mnzsss/skills");
    assert_eq!(meta.source.as_ref().unwrap().path, "skills/lint");
    assert_eq!(meta.source.as_ref().unwrap().git_ref, "main");
}

#[test]
fn add_with_defaults_uses_empty_path_and_ref() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let _report = add(
        &ctx,
        AddInput {
            repo: "owner/toolkit".to_owned(),
            path: None,
            ref_: None,
        },
    )
    .unwrap();

    let skill_dir = Layout::at(root).hall_skills().join("toolkit");
    let source = fs::read_text(&skill_dir.join("SKILL.md")).unwrap().unwrap();
    let meta = parse_frontmatter(&source).unwrap().unwrap();

    assert_eq!(meta.source.as_ref().unwrap().repo, "owner/toolkit");
    assert_eq!(meta.source.as_ref().unwrap().path, "");
    assert_eq!(meta.source.as_ref().unwrap().git_ref, "");
}

#[test]
fn add_is_rejected_for_a_duplicate_skill() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    add(
        &ctx,
        AddInput {
            repo: "owner/toolkit".to_owned(),
            path: None,
            ref_: None,
        },
    )
    .unwrap();

    let failure = add(
        &ctx,
        AddInput {
            repo: "other/toolkit".to_owned(),
            path: None,
            ref_: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "skill.already_exists");
}

#[test]
fn add_derives_id_from_last_path_segment_of_repo() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(
        &ctx,
        AddInput {
            repo: "deep/owner/repo-name".to_owned(),
            path: None,
            ref_: None,
        },
    )
    .unwrap();

    assert_eq!(report.value.id.as_str(), "repo-name");
}
