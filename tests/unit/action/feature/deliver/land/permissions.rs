use super::super::fixture::*;
use super::super::*;

#[test]
fn write_bits_are_restored_when_the_merge_fails() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    std::fs::write(default_worktree.join("uncommitted.txt"), "dirty").unwrap();

    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();
    let preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("preview")
    .value
    .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
        },
    )
    .expect_err("dirty worktree must fail land");
    assert_eq!(failure.code, "deliver.land_dirty_worktree");

    assert_eq!(
        crate::infra::fs::unix_mode(&default_worktree).unwrap(),
        before,
        "a failed land must not leave a read-only repo writable"
    );
}

#[test]
fn write_bits_are_restored_after_successful_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    crate::infra::fs::clear_write_bits(&default_worktree).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("land apply");

    assert!(out.value.land.iter().all(|r| r.merged));
    assert_eq!(
        crate::infra::fs::unix_mode(&default_worktree).unwrap(),
        before,
        "a successful land must restore original read-only permissions"
    );
}

#[test]
fn partial_write_guard_lift_failure_restores_already_lifted_worktree() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::clear_write_bits(&default_worktree).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();

    let unreadable_dir = root.join("unreadable");
    std::fs::create_dir(&unreadable_dir).unwrap();
    let file_inside = unreadable_dir.join("file.txt");
    std::fs::write(&file_inside, "test").unwrap();
    crate::infra::fs::chmod(&unreadable_dir, 0o000).unwrap();

    let result = crate::action::feature::deliver::land::WorktreeWriteGuard::lift(&[
        &default_worktree,
        &file_inside,
    ]);
    assert!(result.is_err(), "lift must fail on inaccessible path");

    // Restore unreadable_dir mode so tempdir cleanup succeeds
    crate::infra::fs::chmod(&unreadable_dir, 0o755).unwrap();

    let after = crate::infra::fs::unix_mode(&default_worktree).unwrap();
    assert_eq!(
        before, after,
        "already lifted worktree must be restored if lift fails on a later worktree"
    );
}

#[test]
fn exact_mode_restoration_preserves_original_permission_bits() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    // Set permission to 0o500 (read+exec owner; group and other have no permissions)
    crate::infra::fs::chmod(&default_worktree, 0o500).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree)
        .unwrap()
        .unwrap();
    assert_eq!(before & 0o777, 0o500);

    {
        let _lifted =
            crate::action::feature::deliver::land::WorktreeWriteGuard::lift(&[&default_worktree])
                .expect("lift");
    }

    let after = crate::infra::fs::unix_mode(&default_worktree)
        .unwrap()
        .unwrap();
    assert_eq!(
        after & 0o777,
        0o500,
        "exact mode bits 0o500 must be restored, not altered to 0o555"
    );
}
