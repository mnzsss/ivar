#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::store::layout::Layout;
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

fn write_skill(root: &camino::Utf8Path, id: &str, source: Option<&str>) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    let source_block = if let Some(repo) = source {
        format!("\nsource:\n  repo: \"{repo}\"\n  path: \"skills/{id}\"\n  ref: \"main\"")
    } else {
        String::new()
    };
    fs::write_text(
        &dir.join("SKILL.md"),
        &format!(
            "---\nname: {id}\ndescription: test skill{src}\n---\n\nbody\n",
            src = source_block
        ),
    )
    .unwrap();
}

#[test]
fn update_authored_skill_is_a_noop_with_warning() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "refactor", None);

    let ctx = Ctx::new(root);
    let report = update(
        &ctx,
        UpdateInput {
            skills: vec!["refactor".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(report.value.processed, 1);
    assert!(!report.warnings.is_empty());
    assert_eq!(report.warnings[0].code, "skill.update.authored_noop");
    assert_eq!(report.warnings[0].subject, "refactor");
}

#[test]
fn update_external_skill_attempts_download() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "external-skill", Some("owner/toolkit"));

    let ctx = Ctx::new(root);
    let report = update(
        &ctx,
        UpdateInput {
            skills: vec!["external-skill".to_owned()],
        },
    )
    .unwrap();

    // The download will fail (no real network), but it should be recorded
    // as a warning, not a hard error.
    assert_eq!(report.value.processed, 1);
    assert!(!report.warnings.is_empty());
    assert_eq!(report.warnings[0].code, "skill.update.download_failed");
}

#[test]
fn one_failing_skill_does_not_abort_the_batch() {
    let (_guard, root) = seeded_hall();
    // Authored skill — no-op (always succeeds)
    write_skill(&root, "authored", None);
    // External skill — will fail to download
    write_skill(&root, "external", Some("owner/toolkit"));

    let ctx = Ctx::new(root);
    let report = update(
        &ctx,
        UpdateInput {
            skills: vec!["authored".to_owned(), "external".to_owned()],
        },
    )
    .unwrap();

    // Both skills were processed.
    assert_eq!(report.value.processed, 2);
    // Two warnings: one authored_noop, one download_failed.
    assert_eq!(report.warnings.len(), 2);
    assert_eq!(report.warnings[0].code, "skill.update.authored_noop");
    assert_eq!(report.warnings[1].code, "skill.update.download_failed");
}

#[test]
fn update_of_nonexistent_skill_is_clean() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = update(
        &ctx,
        UpdateInput {
            skills: vec!["nonexistent".to_owned()],
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.processed, 0);
}
