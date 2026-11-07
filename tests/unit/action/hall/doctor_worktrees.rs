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

// -- orphan classification -------------------------------------------------

use crate::action::hall::{
    BranchPullRequest, DoctorInput, PullRequestLookup, PullRequestState, doctor_with,
};
use crate::test_support::git as run_git;

/// A forge that answers from a fixed list, or panics when asked at all.
enum FakePullRequests {
    Unreachable,
    Answers(Result<Vec<BranchPullRequest>, String>),
}

impl PullRequestLookup for FakePullRequests {
    fn for_branch(
        &self,
        _bare: &Utf8Path,
        _branch: &str,
    ) -> Result<Vec<BranchPullRequest>, String> {
        match self {
            Self::Unreachable => panic!("the doctor reached the forge without being asked"),
            Self::Answers(answer) => answer.clone(),
        }
    }
}

fn pull_request(state: PullRequestState, head: &str) -> BranchPullRequest {
    BranchPullRequest {
        number: 12,
        state,
        head_oid: Some(head.to_owned()),
    }
}

fn origin_of(root: &Utf8Path) -> Utf8PathBuf {
    root.parent().unwrap().join("origins").join("api")
}

/// An orphan worktree `.ivar/repos/api/stray` on `fix/stray` holding one
/// commit that adds `feature.txt`.
fn orphan_with_commit(root: &Utf8Path, bare: &Utf8Path) -> Utf8PathBuf {
    let stray = root.join(".ivar/repos/api/stray");
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, bare, &stray, "fix/stray").unwrap();
    std::fs::write(stray.join("feature.txt"), "work\n").unwrap();
    run_git(&stray, &["add", "feature.txt"]);
    run_git(&stray, &["commit", "-m", "work"]);
    stray
}

/// Land `feature.txt` on the origin's `main` as a new commit — what a
/// squash-merge does — and refresh the bare's remote-tracking refs.
fn squash_merge_into_origin(root: &Utf8Path, bare: &Utf8Path) {
    let origin = origin_of(root);
    std::fs::write(origin.join("feature.txt"), "work\n").unwrap();
    run_git(&origin, &["add", "feature.txt"]);
    run_git(&origin, &["commit", "-m", "squash of fix/stray"]);
    crate::git::Git::fetch(&crate::git::System, bare).unwrap();
}

fn doctor_orphans(root: &Utf8Path, input: DoctorInput, prs: &FakePullRequests) -> DoctorOutcome {
    doctor_with(&Ctx::new(root.to_path_buf()), &input, prs)
        .unwrap()
        .value
}

fn only_orphan(report: &DoctorOutcome) -> &Diagnosis {
    let orphans = orphan_findings(report);
    assert_eq!(orphans.len(), 1, "{orphans:?}");
    orphans[0]
}

#[test]
fn doctor_calls_a_squash_merged_branch_deleted_on_the_remote_safe() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let stray = orphan_with_commit(&root, &bare);
    run_git(&stray, &["push", "origin", "fix/stray"]);
    squash_merge_into_origin(&root, &bare);
    run_git(&origin_of(&root), &["branch", "-D", "fix/stray"]);
    crate::git::Git::fetch(&crate::git::System, &bare).unwrap();

    let report = doctor_orphans(
        &root,
        DoctorInput::default(),
        &FakePullRequests::Unreachable,
    );

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("safe to remove"), "{}", orphan.what);
    assert!(
        orphan.what.contains("no uncommitted changes"),
        "{}",
        orphan.what
    );
    assert!(orphan.what.contains("no remote branch"), "{}", orphan.what);
    assert!(
        orphan.what.contains("already match `main`"),
        "{}",
        orphan.what
    );
    assert!(orphan.fix.contains("ivar doctor --fix"), "{}", orphan.fix);
}

#[test]
fn doctor_calls_a_branch_with_commits_main_lacks_a_look() {
    let (_guard, root, bare) = synced_hall_with_bare();
    orphan_with_commit(&root, &bare);

    let report = doctor_orphans(
        &root,
        DoctorInput::default(),
        &FakePullRequests::Unreachable,
    );

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("needs a look"), "{}", orphan.what);
    assert!(
        orphan.what.contains("differ from `main`"),
        "{}",
        orphan.what
    );
    assert!(!orphan.fix.contains("--fix"), "{}", orphan.fix);
}

#[test]
fn doctor_never_calls_an_orphan_with_uncommitted_changes_safe_or_removes_it() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let stray = root.join(".ivar/repos/api/stray");
    let git = crate::git::System;
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();
    std::fs::write(stray.join("scratch.txt"), "mine\n").unwrap();
    std::fs::write(stray.join("README.md"), "edited\n").unwrap();

    let input = DoctorInput {
        github: false,
        fix: true,
    };
    let report = doctor_orphans(&root, input, &FakePullRequests::Unreachable);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("needs a look"), "{}", orphan.what);
    assert!(
        orphan.what.contains("2 uncommitted changes"),
        "{}",
        orphan.what
    );
    assert!(report.removed.is_empty());
    assert!(stray.join("scratch.txt").exists());
}

#[test]
fn doctor_never_calls_an_orphan_with_unpushed_commits_safe_even_when_main_has_the_files() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let stray = orphan_with_commit(&root, &bare);
    run_git(&stray, &["push", "origin", "fix/stray"]);
    std::fs::write(stray.join("later.txt"), "later\n").unwrap();
    run_git(&stray, &["add", "later.txt"]);
    run_git(&stray, &["commit", "-m", "later"]);
    // Main ends up with both files, so a file comparison alone would pass.
    let origin = origin_of(&root);
    std::fs::write(origin.join("feature.txt"), "work\n").unwrap();
    std::fs::write(origin.join("later.txt"), "later\n").unwrap();
    run_git(&origin, &["add", "."]);
    run_git(&origin, &["commit", "-m", "squash"]);
    crate::git::Git::fetch(&crate::git::System, &bare).unwrap();

    let input = DoctorInput {
        github: false,
        fix: true,
    };
    let report = doctor_orphans(&root, input, &FakePullRequests::Unreachable);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("needs a look"), "{}", orphan.what);
    assert!(
        orphan
            .what
            .contains("1 commit not pushed to `origin/fix/stray`"),
        "{}",
        orphan.what
    );
    assert!(report.removed.is_empty());
    assert!(stray.exists());
}

#[test]
fn doctor_fix_removes_the_safe_orphans_and_only_those() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let clean = root.join(".ivar/repos/api/clean");
    crate::git::Git::create_branch(&git, &bare, "fix/clean", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &clean, "fix/clean").unwrap();
    let work = root.join(".ivar/repos/api/work");
    crate::git::Git::create_branch(&git, &bare, "fix/work", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &work, "fix/work").unwrap();
    std::fs::write(work.join("new.txt"), "unsaved\n").unwrap();

    let input = DoctorInput {
        github: false,
        fix: true,
    };
    let report = doctor_orphans(&root, input, &FakePullRequests::Unreachable);

    assert_eq!(report.removed, vec![clean.clone()]);
    assert!(!clean.exists());
    assert!(work.join("new.txt").exists());
    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("fix/work"), "{}", orphan.what);
    let mut human = Vec::new();
    report.write_human(&mut human).unwrap();
    assert!(String::from_utf8(human).unwrap().contains(clean.as_str()));
}

#[test]
fn doctor_fix_removes_a_read_only_safe_orphan() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();
    crate::infra::fs::clear_write_bits(&stray).unwrap();

    let input = DoctorInput {
        github: false,
        fix: true,
    };
    let report = doctor_orphans(&root, input, &FakePullRequests::Unreachable);

    assert_eq!(report.removed, vec![stray.clone()]);
    assert!(!stray.exists());
}

#[test]
fn doctor_without_fix_removes_nothing() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let git = crate::git::System;
    let stray = root.join(".ivar/repos/api/stray");
    crate::git::Git::create_branch(&git, &bare, "fix/stray", "main").unwrap();
    crate::git::Git::add_worktree(&git, &bare, &stray, "fix/stray").unwrap();

    let report = doctor_orphans(
        &root,
        DoctorInput::default(),
        &FakePullRequests::Unreachable,
    );

    assert!(report.removed.is_empty());
    assert!(stray.exists());
    assert!(only_orphan(&report).what.contains("safe to remove"));
}

#[test]
fn doctor_trusts_a_merged_pull_request_at_the_branch_head_when_asked() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let stray = orphan_with_commit(&root, &bare);
    let head = crate::git::Git::head_commit(&crate::git::System, &stray).unwrap();
    let forge = FakePullRequests::Answers(Ok(vec![pull_request(PullRequestState::Merged, &head)]));
    let input = DoctorInput {
        github: true,
        fix: false,
    };

    let report = doctor_orphans(&root, input, &forge);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("safe to remove"), "{}", orphan.what);
    assert!(orphan.what.contains("PR #12 merged"), "{}", orphan.what);
}

#[test]
fn doctor_does_not_trust_a_pull_request_merged_at_another_commit() {
    let (_guard, root, bare) = synced_hall_with_bare();
    orphan_with_commit(&root, &bare);
    let forge = FakePullRequests::Answers(Ok(vec![pull_request(
        PullRequestState::Merged,
        "0000000000000000000000000000000000000000",
    )]));
    let input = DoctorInput {
        github: true,
        fix: true,
    };

    let report = doctor_orphans(&root, input, &forge);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("needs a look"), "{}", orphan.what);
    assert!(
        orphan.what.contains("merged at a different commit"),
        "{}",
        orphan.what
    );
    assert!(report.removed.is_empty());
}

#[test]
fn doctor_reports_a_closed_pull_request_without_calling_it_integrated() {
    let (_guard, root, bare) = synced_hall_with_bare();
    let stray = orphan_with_commit(&root, &bare);
    let head = crate::git::Git::head_commit(&crate::git::System, &stray).unwrap();
    let forge = FakePullRequests::Answers(Ok(vec![pull_request(PullRequestState::Closed, &head)]));
    let input = DoctorInput {
        github: true,
        fix: false,
    };

    let report = doctor_orphans(&root, input, &forge);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("needs a look"), "{}", orphan.what);
    assert!(
        orphan.what.contains("PR #12 closed without merging"),
        "{}",
        orphan.what
    );
}

#[test]
fn doctor_falls_back_to_the_local_comparison_when_the_forge_cannot_be_asked() {
    let (_guard, root, bare) = synced_hall_with_bare();
    orphan_with_commit(&root, &bare);
    squash_merge_into_origin(&root, &bare);
    let forge = FakePullRequests::Answers(Err("`gh` is not installed".to_owned()));
    let input = DoctorInput {
        github: true,
        fix: false,
    };

    let report = doctor_orphans(&root, input, &forge);

    let orphan = only_orphan(&report);
    assert!(orphan.what.contains("safe to remove"), "{}", orphan.what);
    assert!(
        orphan.what.contains("`gh` is not installed"),
        "{}",
        orphan.what
    );
}

#[test]
fn doctor_names_the_feature_that_owned_a_folder_named_for_another_branch() {
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
    crate::git::Git::create_branch(&git, &bare, "fix/other", "main").unwrap();
    crate::git::Git::add_worktree(
        &git,
        &bare,
        &root.join(".ivar/repos/api/checkout"),
        "fix/other",
    )
    .unwrap();

    let report = doctor(&ctx).unwrap().value;

    let orphan = only_orphan(&report);
    assert!(
        orphan.what.contains("feature `checkout`"),
        "{}",
        orphan.what
    );
}

#[test]
fn doctor_does_not_name_an_owner_when_folder_and_branch_agree() {
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

    let report = doctor(&ctx).unwrap().value;

    assert!(!only_orphan(&report).what.contains("the folder was feature"));
}
