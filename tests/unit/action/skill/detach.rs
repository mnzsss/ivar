#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn write_external_skill(root: &camino::Utf8Path, id: &str) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    let content = "---\nname: ext-skill\ndescription: An external skill\nsource:\n  repo: owner/repo\n  path: skills/ext\n  ref: main\n---\n\nThis is the body.\nIt should stay.\n";
    fs::write_text(&dir.join("SKILL.md"), content).unwrap();
}

fn write_authored_skill(root: &camino::Utf8Path, id: &str) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    let content = "---\nname: auth-skill\ndescription: An authored skill\n---\n\nBody only.\n";
    fs::write_text(&dir.join("SKILL.md"), content).unwrap();
}

#[test]
fn detach_removes_source_from_frontmatter_and_preserves_body() {
    let (_guard, root) = seeded_hall();
    write_external_skill(&root, "detach_me");
    let ctx = Ctx::new(root.clone());

    detach(
        &ctx,
        DetachInput {
            skill: "detach_me".to_owned(),
        },
    )
    .unwrap();

    // Verify the source field is gone.
    let raw = fs::read_text(
        &root
            .join(".ivar")
            .join("skills")
            .join("detach_me")
            .join("SKILL.md"),
    )
    .unwrap()
    .unwrap();

    // Parse frontmatter and check no source.
    let fm = skill::parse_frontmatter(&raw).unwrap().unwrap();
    assert!(fm.source.is_none(), "source should be removed after detach");
    assert_eq!(fm.name, "ext-skill");
    assert_eq!(fm.description, Some("An external skill".to_owned()));
}

#[test]
fn detach_of_authored_is_a_no_op() {
    let (_guard, root) = seeded_hall();
    write_authored_skill(&root, "authored");
    let ctx = Ctx::new(root.clone());

    let result = detach(
        &ctx,
        DetachInput {
            skill: "authored".to_owned(),
        },
    );

    // Returns success (no error) — it's a no-op.
    result.unwrap();

    // Frontmatter unchanged (still no source).
    let raw = fs::read_text(
        &root
            .join(".ivar")
            .join("skills")
            .join("authored")
            .join("SKILL.md"),
    )
    .unwrap()
    .unwrap();
    let fm = skill::parse_frontmatter(&raw).unwrap().unwrap();
    assert!(fm.source.is_none());
}

#[test]
fn detach_rejects_a_nonexistent_skill() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = detach(
        &ctx,
        DetachInput {
            skill: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "skill.not_found");
}

#[test]
fn detach_preserves_body_text_unchanged() {
    let (_guard, root) = seeded_hall();
    write_external_skill(&root, "body_test");
    let ctx = Ctx::new(root.clone());

    detach(
        &ctx,
        DetachInput {
            skill: "body_test".to_owned(),
        },
    )
    .unwrap();

    let raw = fs::read_text(
        &root
            .join(".ivar")
            .join("skills")
            .join("body_test")
            .join("SKILL.md"),
    )
    .unwrap()
    .unwrap();
    // Body should contain the original text.
    assert!(raw.contains("This is the body."));
    assert!(raw.contains("It should stay."));
}
