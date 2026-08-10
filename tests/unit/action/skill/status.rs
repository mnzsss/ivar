#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

fn write_skill(root: &camino::Utf8Path, id: &str, is_external: bool) {
    let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
    fs::ensure_dir(&dir).unwrap();
    if is_external {
        let content = "---\nname: ext\nsource:\n  repo: owner/repo\n  path: skills/ext\n  ref: main\n---\n\nBody.\n";
        fs::write_text(&dir.join("SKILL.md"), content).unwrap();
    } else {
        let content = "---\nname: auth\ndescription: Authored skill\n---\n\nBody.\n";
        fs::write_text(&dir.join("SKILL.md"), content).unwrap();
    }
}

#[test]
fn status_reports_authored_and_external_skills() {
    let (_guard, root) = seeded_hall();
    write_skill(&root, "authed", false);
    write_skill(&root, "ext", true);
    let ctx = Ctx::new(root);

    let result = status(&ctx);
    assert!(result.is_ok());
}

#[test]
fn status_handles_an_empty_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let result = status(&ctx);
    assert!(result.is_ok());
}

#[test]
fn material_status_label_maps_all_variants() {
    assert_eq!(material_status_label(MaterialStatus::Missing), "missing");
    assert_eq!(material_status_label(MaterialStatus::Ok), "ok");
    assert_eq!(
        material_status_label(MaterialStatus::WrongLink),
        "wrong link"
    );
    assert_eq!(
        material_status_label(MaterialStatus::NotLink),
        "not a symlink"
    );
    assert_eq!(
        material_status_label(MaterialStatus::BrokenSymlink),
        "broken symlink"
    );
}

#[test]
fn the_human_surface_lists_skills_with_kind_markers() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        skills: vec![
            SkillStatus {
                id: RepoName::new("audit").unwrap(),
                source: "authored".to_owned(),
                targets: vec![TargetStatus {
                    target: "claude".to_owned(),
                    status: "ok".to_owned(),
                }],
            },
            SkillStatus {
                id: RepoName::new("lint").unwrap(),
                source: "external".to_owned(),
                targets: vec![TargetStatus {
                    target: "opencode".to_owned(),
                    status: "missing".to_owned(),
                }],
            },
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("[A] audit"));
    assert!(text.contains("[E] lint"));
    assert!(text.contains("claude: ok"));
    assert!(text.contains("opencode: missing"));
}
