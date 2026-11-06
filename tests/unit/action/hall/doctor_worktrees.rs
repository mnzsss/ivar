use super::*;

#[test]
fn doctor_names_a_worktree_no_feature_owns() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    let bare = root.join(".ivar/repos/api/.bare");
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/stray"),
        "fix/stray",
    )
    .unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "repo.worktree_orphaned");
    assert!(finding.what.contains("stray"));
    assert!(finding.what.contains("fix/stray"));
    assert!(!finding.fix.contains("--force"));
}

#[test]
fn doctor_names_a_worktree_on_a_feature_branch_in_a_repo_it_did_not_promote() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let ctx = Ctx::new(root.clone());
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "checkout", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/checkout"),
        "checkout",
    )
    .unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        finding(&report.value, "repo.worktree_orphaned")
            .what
            .contains("`checkout`")
    );
}

#[test]
fn doctor_reports_an_unreadable_feature_record_instead_of_orphaning_its_worktree() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let ctx = Ctx::new(root.clone());
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "checkout", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/checkout"),
        "checkout",
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    let record = layout
        .feature_dir(&FeatureName::new("checkout").unwrap())
        .join("feature.json");
    std::fs::write(&record, "{ not json").unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        finding(&report.value, "feature.record_unreadable")
            .what
            .contains("checkout")
    );
    assert!(orphan_findings(&report.value).is_empty());
}

/// A feature created with `--base develop` owns the `develop` base worktree
/// its sessions link; once the feature is deleted nothing owns it.
#[test]
fn doctor_counts_a_live_features_base_worktree_as_owned() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let ctx = Ctx::new(root.clone());
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: Some("develop".to_owned()),
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "develop", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/develop"),
        "develop",
    )
    .unwrap();

    let live = doctor(&ctx).unwrap();

    assert!(
        orphan_findings(&live.value).is_empty(),
        "{:?}",
        orphan_findings(&live.value)
    );

    crate::action::feature::delete::delete(
        &ctx,
        crate::action::feature::delete::DeleteInput {
            name: "checkout".to_owned(),
            force: false,
            descendants: crate::action::feature::delete::Descendants::Consented,
        },
    )
    .unwrap();

    let deleted = doctor(&ctx).unwrap();

    let orphans = orphan_findings(&deleted.value);
    assert_eq!(orphans.len(), 1, "{orphans:?}");
    assert!(orphans[0].what.contains("`develop`"), "{}", orphans[0].what);
}

#[test]
fn doctor_names_a_stray_detached_worktree() {
    let (_guard, root, bare) = synced_hall_with_bare();
    crate::git::Git::add_detached_worktree(
        &crate::git::System,
        &bare,
        &root.join(".ivar/repos/api/loose"),
        "main",
    )
    .unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let finding = finding(&report.value, "repo.worktree_orphaned");
    assert!(finding.what.contains("loose"));
    assert!(finding.what.contains("detached"));
}

#[test]
fn doctor_leaves_an_integration_candidate_alone() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let candidate = Layout::at(root.clone()).integration_candidate(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::RepoName::new("api").unwrap(),
    );
    crate::git::Git::add_detached_worktree(&crate::git::System, &bare, &candidate, "main").unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(orphan_findings(&report.value).is_empty());
}

#[test]
fn doctor_leaves_an_integration_source_worktree_alone() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let source = Layout::at(root.clone()).integration_source(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::RepoName::new("api").unwrap(),
    );
    crate::git::Git::create_branch(&git, &bare, "ivar-integrate/checkout/api", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &source, "ivar-integrate/checkout/api").unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(orphan_findings(&report.value).is_empty());
}

#[test]
fn doctor_suggests_sync_for_a_worktree_whose_directory_is_gone() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();
    std::fs::remove_dir_all(&stray).unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let finding = finding(&report.value, "repo.worktree_orphaned");
    assert!(finding.what.contains("its directory is gone"));
    assert!(!finding.what.contains("uncommitted"));
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_missing_bare_clone_and_its_fix() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "repo.bare_missing");
    assert!(finding.fix.contains("ivar sync"));
}

fn unregistered_findings(report: &DoctorOutcome) -> Vec<&Diagnosis> {
    report
        .findings
        .iter()
        .filter(|finding| finding.code == "repo.worktree_unregistered")
        .collect()
}

#[test]
fn doctor_tells_the_user_to_lift_the_guard_on_a_read_only_orphan() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();
    crate::infra::fs::clear_write_bits(&stray).unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let orphans = orphan_findings(&report.value);
    assert_eq!(orphans.len(), 1, "{orphans:?}");
    let chmod = format!("chmod u+w {stray} && git --git-dir {bare} worktree remove {stray}");
    assert!(orphans[0].fix.contains(&chmod), "{}", orphans[0].fix);
}

#[test]
fn doctor_does_not_mention_chmod_for_a_writable_orphan() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        !finding(&report.value, "repo.worktree_orphaned")
            .fix
            .contains("chmod")
    );
}

/// The failure this finding exists for: git removing a guarded checkout
/// unregisters it, then cannot delete the files.
#[test]
fn doctor_names_the_directory_a_failed_worktree_remove_left_behind() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();
    crate::infra::fs::clear_write_bits(&stray).unwrap();
    let removal = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str(), "worktree", "remove", "--force"])
        .arg(stray.as_str())
        .output()
        .unwrap();
    // Running as root, permission bits do not stop git; there is nothing to reproduce.
    if removal.status.success() {
        return;
    }
    assert!(stray.join(".git").exists());

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(orphan_findings(&report.value).is_empty());
    let unregistered = unregistered_findings(&report.value);
    assert_eq!(unregistered.len(), 1, "{unregistered:?}");
    assert!(unregistered[0].what.contains("stray"));
    assert!(unregistered[0].fix.contains(&format!("chmod u+w {stray}")));
    assert!(unregistered[0].fix.contains(&format!("rm -rf {stray}")));
    assert!(!unregistered[0].fix.contains("worktree remove"));
}

#[test]
fn doctor_names_an_unregistered_checkout_under_a_branch_prefix() {
    let (_guard, root, _bare) = synced_hall_with_bare();
    let leftover = root.join(".ivar/repos/api/fix/leftover");
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join(".git"), "gitdir: /nowhere\n").unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let unregistered = unregistered_findings(&report.value);
    assert_eq!(unregistered.len(), 1, "{unregistered:?}");
    assert!(unregistered[0].what.contains("fix/leftover"));
    assert!(!unregistered[0].fix.contains("chmod"));
}

#[test]
fn doctor_does_not_flag_registered_checkouts_or_branch_prefixes_as_unregistered() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let ctx = Ctx::new(root.clone());
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: Some("feat/checkout".to_owned()),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "feat/checkout", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/feat/checkout"),
        "feat/checkout",
    )
    .unwrap();
    // A prefix directory that holds no checkout, and the bare itself.
    std::fs::create_dir_all(root.join(".ivar/repos/api/empty/prefix")).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        unregistered_findings(&report.value).is_empty(),
        "{:?}",
        report.value.findings
    );
}
