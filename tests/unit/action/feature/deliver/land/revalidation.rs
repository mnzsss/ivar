use super::super::fixture::*;
use super::super::*;

#[test]
fn land_revalidates_head_movement_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_feature = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );

    // Make web_default at commit C2, and web_feature at C2 -> work
    git(&web_feature, &["commit", "--allow-empty", "-m", "c2"]);
    let c2 = git_stdout(&web_feature, &["rev-parse", "HEAD"]);
    git(&web_feature, &["commit", "--allow-empty", "-m", "work2"]);
    git(&web_default, &["reset", "--hard", c2.trim()]);

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api moves web_default HEAD between preflight and phase 2
    let manifest = read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone()
                    .with_checks(vec![format!("git -C {web_default} reset --hard HEAD~1")])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest =
        Manifest::new(manifest.name().clone(), manifest.providers().clone(), repos).unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("HEAD movement during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_head_moved");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);
}

#[test]
fn land_revalidates_dirty_worktree_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api dirties web_default worktree between preflight and phase 2
    let manifest = read_manifest(&layout).unwrap();
    let dirty_path = web_default.join("dirty_test.txt");
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone()
                    .with_checks(vec![format!("sh -c 'echo dirty > {dirty_path}'")])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest =
        Manifest::new(manifest.name().clone(), manifest.providers().clone(), repos).unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("dirty worktree during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_dirty_worktree");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);

    // Repo 2 dirty worktree is untouched
    assert!(dirty_path.exists());
    assert_eq!(std::fs::read_to_string(&dirty_path).unwrap(), "dirty\n");
}

#[test]
fn land_revalidates_fast_forward_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api creates a non-fast-forward commit on web_default
    let manifest = read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                // Stands in for someone else moving the default branch between
                // preview and apply. `core.hooksPath=` because this is staging
                // that divergence, not exercising the protection hook.
                r.clone().with_checks(vec![format!(
                    "git -c core.hooksPath= -C {web_default} commit --allow-empty -m 'diverged'"
                )])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest =
        Manifest::new(manifest.name().clone(), manifest.providers().clone(), repos).unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("non-fast-forward during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_not_fast_forward");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);
}

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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
