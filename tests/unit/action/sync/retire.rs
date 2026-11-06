//! What `sync` does with the checkout a repo's previous default branch left
//! behind after `defaultBranch` changes in `ivar.json`.

use super::*;
use crate::action::feature::create::{CreateInput, create};
use crate::git::Git as _;

/// A synced hall whose repo `api` defaults to `develop`, with a `main` branch
/// at the same commit. The `develop` checkout carries the read-only guard a
/// session puts on a default checkout.
fn hall_defaulting_to_develop() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with(&[("api", "develop")]);
    let origin = root.parent().unwrap().join("origins").join("api");
    crate::test_support::git(&origin, &["branch", "main"]);
    sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();
    fs::clear_write_bits(&root.join(".ivar/repos/api/develop")).unwrap();
    (guard, root)
}

fn set_default_branch(root: &Utf8Path, branch: &str) {
    let origin = root.parent().unwrap().join("origins").join("api");
    let layout = Layout::at(root);
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new(branch).unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
}

fn checkout_exists(root: &Utf8Path, branch: &str) -> bool {
    let bare = root.join(".ivar/repos/api/.bare");
    crate::git::System
        .list_worktrees(&bare)
        .unwrap()
        .iter()
        .any(|entry| entry.branch.as_deref() == Some(branch))
        && root.join(format!(".ivar/repos/api/{branch}")).exists()
}

fn kept_warnings(outcome: &crate::error::Report<SyncOutcome>) -> Vec<&crate::error::Warning> {
    outcome
        .warnings
        .iter()
        .filter(|warning| warning.code == "sync.stale_default_checkout_kept")
        .collect()
}

#[test]
fn sync_retires_a_clean_contained_checkout_of_the_old_default() {
    let (_guard, root) = hall_defaulting_to_develop();
    set_default_branch(&root, "main");

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(
        entry(&report.value, "repo api", "worktree develop").change,
        Change::Removed
    );
    assert!(!root.join(".ivar/repos/api/develop").exists());
    assert!(checkout_exists(&root, "main"));
    // The worktree goes; the branch stays.
    let bare = root.join(".ivar/repos/api/.bare");
    assert!(
        crate::git::System
            .list_branches(&bare)
            .unwrap()
            .contains(&"develop".to_owned())
    );
}

#[test]
fn a_second_sync_after_the_retirement_changes_nothing() {
    let (_guard, root) = hall_defaulting_to_develop();
    set_default_branch(&root, "main");
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(
        report
            .value
            .entries
            .iter()
            .all(|e| e.change == Change::Unchanged),
        "{:?}",
        report.value.entries
    );
}

#[test]
fn sync_warns_and_keeps_an_old_default_checkout_with_uncommitted_changes() {
    let (_guard, root) = hall_defaulting_to_develop();
    let stale = root.join(".ivar/repos/api/develop");
    fs::chmod(&stale, 0o755).unwrap();
    std::fs::write(stale.join("README.md"), "work in progress\n").unwrap();
    fs::clear_write_bits(&stale).unwrap();
    set_default_branch(&root, "main");

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    let kept = kept_warnings(&report);
    assert_eq!(kept.len(), 1, "{:?}", report.warnings);
    assert!(kept[0].what.contains("uncommitted"), "{}", kept[0].what);
    assert!(checkout_exists(&root, "develop"));
    assert_eq!(
        std::fs::read_to_string(stale.join("README.md")).unwrap(),
        "work in progress\n"
    );
}

#[test]
fn sync_warns_and_keeps_an_old_default_checkout_with_commits_the_new_default_lacks() {
    let (_guard, root) = hall_defaulting_to_develop();
    let stale = root.join(".ivar/repos/api/develop");
    fs::chmod(&stale, 0o755).unwrap();
    std::fs::write(stale.join("extra.txt"), "only on develop\n").unwrap();
    crate::test_support::git(&stale, &["add", "extra.txt"]);
    crate::test_support::git(&stale, &["commit", "-m", "develop only"]);
    fs::clear_write_bits(&stale).unwrap();
    set_default_branch(&root, "main");

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    let kept = kept_warnings(&report);
    assert_eq!(kept.len(), 1, "{:?}", report.warnings);
    assert!(
        kept[0].what.contains("does not contain"),
        "{}",
        kept[0].what
    );
    assert!(checkout_exists(&root, "develop"));
}

/// A commit that reached the new default as a rebase or squash is contained by
/// content, not by ancestry.
#[test]
fn sync_retires_an_old_default_checkout_whose_commits_were_cherry_picked() {
    let (_guard, root) = hall_defaulting_to_develop();
    let stale = root.join(".ivar/repos/api/develop");
    fs::chmod(&stale, 0o755).unwrap();
    std::fs::write(stale.join("extra.txt"), "landed twice\n").unwrap();
    crate::test_support::git(&stale, &["add", "extra.txt"]);
    crate::test_support::git(&stale, &["commit", "-m", "landed"]);
    fs::clear_write_bits(&stale).unwrap();
    // Re-land the same change on `main` as a separate commit, without
    // touching a checkout: build it in a scratch clone and push it.
    let bare = root.join(".ivar/repos/api/.bare");
    let scratch = root.parent().unwrap().join("scratch");
    crate::test_support::git(
        root.parent().unwrap(),
        &["clone", "--branch", "main", bare.as_str(), scratch.as_str()],
    );
    std::fs::write(scratch.join("extra.txt"), "landed twice\n").unwrap();
    crate::test_support::git(&scratch, &["add", "extra.txt"]);
    crate::test_support::git(&scratch, &["commit", "-m", "landed again"]);
    crate::test_support::git(&scratch, &["push", "origin", "main"]);
    set_default_branch(&root, "main");

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert!(!root.join(".ivar/repos/api/develop").exists());
}

#[test]
fn sync_leaves_an_unguarded_extra_checkout_alone() {
    let (_guard, root) = hall_defaulting_to_develop();
    let bare = root.join(".ivar/repos/api/.bare");
    crate::git::System
        .create_branch(&bare, "fix/stray", "main")
        .unwrap();
    crate::git::System
        .add_worktree(&bare, &root.join(".ivar/repos/api/stray"), "fix/stray")
        .unwrap();
    set_default_branch(&root, "main");

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    assert!(root.join(".ivar/repos/api/stray").exists());
    assert!(
        !report
            .value
            .entries
            .iter()
            .any(|e| e.label == "worktree fix/stray"),
        "{:?}",
        report.value.entries
    );
}

/// `--base develop` makes `develop` a live feature's base worktree, and it is
/// guarded the same way as a default checkout. It is the feature's, not stale.
#[test]
fn sync_leaves_a_live_features_base_worktree_alone() {
    let (_guard, root) = hall_defaulting_to_develop();
    let ctx = Ctx::new(root.clone());
    create(
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
    set_default_branch(&root, "main");

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert!(kept_warnings(&report).is_empty());
    assert!(
        !report
            .value
            .entries
            .iter()
            .any(|e| e.label == "worktree develop" && e.change == Change::Removed),
        "{:?}",
        report.value.entries
    );
    assert!(checkout_exists(&root, "develop"));
}
