#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
use crate::store::layout::Layout;
use crate::test_support::{git, hall_root};

fn hall_with_origin() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
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
    git(&root, &["init", "--initial-branch", "main", "-q"]);
    let origin = root.parent().unwrap().join("origins/hall.git");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--bare", "-q"]);
    git(&root, &["remote", "add", "origin", origin.as_str()]);
    (guard, root, origin)
}

fn local(name: &str) -> CreateInput {
    CreateInput {
        name: name.to_owned(),
        mode: CreateMode::Local,
        default_branch: None,
    }
}

fn remote_refs(git_dir: &Utf8Path) -> String {
    let out = std::process::Command::new("git")
        .args([
            "--git-dir",
            git_dir.as_str(),
            "for-each-ref",
            "--format=%(refname)",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn create_local_pushes_a_readme_under_the_prefix_and_registers_the_repo() {
    let (_guard, root, origin) = hall_with_origin();
    let ctx = Ctx::new(root.clone());

    let report = create(&ctx, local("notes")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.ref_prefix.as_deref(), Some("repos/notes/"));
    assert_eq!(remote_refs(&origin).trim(), "refs/heads/repos/notes/main");
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/notes/main/README.md")).unwrap(),
        "# notes\n"
    );
    let manifest = crate::action::read_manifest(&Layout::at(root.clone())).unwrap();
    assert_eq!(manifest.repos()[0].url(), origin.as_str());
    assert_eq!(manifest.repos()[0].ref_prefix(), Some("repos/notes/"));
}

#[test]
fn create_local_in_a_hall_without_origin_is_refused_before_any_write() {
    let (_guard, root, _) = hall_with_origin();
    git(&root, &["remote", "remove", "origin"]);
    let ctx = Ctx::new(root.clone());

    let error = create(&ctx, local("notes")).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "repo.hall_has_no_origin");
    assert!(!root.join(".ivar/repos/notes").exists());
}

#[test]
fn create_local_refuses_a_name_already_in_the_manifest_without_pushing() {
    let (_guard, root, origin) = hall_with_origin();
    let ctx = Ctx::new(root.clone());
    create(&ctx, local("notes")).unwrap();
    let before = remote_refs(&origin);

    let error = create(&ctx, local("notes")).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(remote_refs(&origin), before);
}
