#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::hall::{self, InitInput};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};

/// A hall with one seeded repo declared, and a feature created.
fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    create_action(
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

    // Materialise the bare clone, the way `ivar sync` would after a
    // `git pull` — promote operates on the cloned repo, never clones.
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    (guard, root)
}

fn promote_input(feature: &str, repo: &str) -> PromoteInput {
    PromoteInput {
        feature: feature.to_owned(),
        repo: repo.to_owned(),
        base: None,
    }
}

#[test]
fn promote_creates_the_branch_and_worktree_from_the_default_branch() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.repo.as_str(), "api");
    assert_eq!(report.value.branch, "checkout");
    // The worktree materialised with the seeded content.
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/checkout/README.md")).unwrap(),
        "seed\n"
    );
    // The promotion record says Ready.
    let feature = Feature::read(
        &Layout::at(root.clone()),
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        feature.worktree_state(&RepoName::new("api").unwrap()),
        Some(WorktreeState::Ready)
    );
}

#[test]
fn promote_creates_the_branch_off_the_default_branch_not_any_other() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());

    promote(&ctx, promote_input("checkout", "api")).unwrap();

    // The feature branch's tip is the default branch's tip.
    let bare = root.join(".ivar/repos/api/.bare");
    let branch_tip = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str(), "rev-parse", "checkout"])
        .output()
        .unwrap();
    let default_tip = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str(), "rev-parse", "main"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&branch_tip.stdout),
        String::from_utf8_lossy(&default_tip.stdout)
    );
}

#[test]
fn promote_is_rejected_when_the_feature_does_not_exist() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = promote(&ctx, promote_input("ghost", "api")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn promote_is_rejected_when_the_repo_is_not_in_the_manifest() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = promote(&ctx, promote_input("checkout", "ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.not_in_manifest");
}

#[test]
fn promote_is_rejected_when_the_repo_is_already_promoted() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    promote(&ctx, promote_input("checkout", "api")).unwrap();

    let failure = promote(&ctx, promote_input("checkout", "api")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.already_promoted");
}

#[test]
fn a_setup_script_runs_in_the_feature_worktree_with_worktree_kind_feature() {
    let (_guard, root) = hall_with_feature();
    let script = Layout::at(root.clone()).setup_script(&RepoName::new("api").unwrap());
    fs::ensure_dir(script.parent().unwrap()).unwrap();
    fs::write_text(
        &script,
        "#!/usr/bin/env bash\nset -euo pipefail\n\
         printf '%s %s %s\\n' \"$IVAR_REPO\" \"$IVAR_BRANCH\" \"$IVAR_WORKTREE_KIND\" > .ivar-setup-ran\n",
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(report.value.setup_ran);
    let evidence = root.join(".ivar/repos/api/checkout/.ivar-setup-ran");
    assert_eq!(
        std::fs::read_to_string(&evidence).unwrap(),
        "api checkout feature\n"
    );
}

/// `IVAR_FEATURE` is the variable `ARCHITECTURE.md` promises on feature
/// worktrees and only there, and `IVAR_SECRETS_DIR` is what a script reads
/// values from that git does not carry.
#[test]
fn a_setup_script_on_promote_gets_the_feature_name_and_the_secrets_dir() {
    let (_guard, root) = hall_with_feature();
    let script = Layout::at(root.clone()).setup_script(&RepoName::new("api").unwrap());
    fs::ensure_dir(script.parent().unwrap()).unwrap();
    fs::write_text(
        &script,
        "#!/usr/bin/env bash\nset -euo pipefail\n\
         printf '%s\\n%s\\n' \"$IVAR_FEATURE\" \"$IVAR_SECRETS_DIR\" > .ivar-env\n",
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    promote(&ctx, promote_input("checkout", "api")).unwrap();

    let evidence =
        std::fs::read_to_string(root.join(".ivar/repos/api/checkout/.ivar-env")).unwrap();
    let mut lines = evidence.lines();
    assert_eq!(lines.next().unwrap(), "checkout");
    assert!(
        lines.next().unwrap().ends_with("/.ivar/secrets"),
        "was: {evidence}"
    );
}

#[test]
fn the_human_surface_names_what_was_promoted() {
    let outcome = PromoteOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        repo: RepoName::new("api").unwrap(),
        branch: "checkout".to_owned(),
        adopted_branch: false,
        setup_ran: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Promoted `api` onto feature `checkout` (branch: checkout)\n"
    );
}

/// Adoption and creation leave the worktree at different commits, so the
/// surface has to say which happened.
#[test]
fn the_human_surface_says_when_a_branch_was_adopted() {
    let outcome = PromoteOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        repo: RepoName::new("api").unwrap(),
        branch: "checkout".to_owned(),
        adopted_branch: true,
        setup_ran: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Promoted `api` onto feature `checkout` (adopted existing branch: checkout)\n"
    );
}

/// The gap this closes: a branch git already has — pushed by a teammate,
/// left by a deleted feature, carried in from another tool — used to make
/// promotion fail outright, because `git worktree add -b` refuses it.
#[test]
fn promote_adopts_a_branch_that_already_exists() {
    let (_guard, root) = hall_with_feature();
    let bare = Layout::at(root.clone()).repo_bare(&RepoName::new("api").unwrap());
    git(&bare, &["branch", "checkout", "main"]);
    let tip = rev_parse(&bare, "checkout");
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(report.value.adopted_branch);
    let worktree = root.join(".ivar/repos/api/checkout");
    assert!(fs::is_dir(&worktree).unwrap());
    assert_eq!(head_branch(&worktree), "checkout");
    // Checked out where the branch already pointed. Adoption must not move
    // the ref — those commits are someone's work.
    assert_eq!(rev_parse(&worktree, "HEAD"), tip);
}

/// The unchanged path, asserted next to the new one so a future edit
/// cannot quietly make adoption the only behaviour.
#[test]
fn promote_still_creates_a_branch_that_does_not_exist() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(!report.value.adopted_branch);
    assert_eq!(
        head_branch(&root.join(".ivar/repos/api/checkout")),
        "checkout"
    );
}

/// A hall with two branches in the seeded repo — `main`, and `develop`,
/// which carries a commit `main` does not have — and a feature created with
/// `base` as given. No repo promoted yet.
fn hall_with_two_branches(base: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    git(&origin, &["add", "develop-only.txt"]);
    git(&origin, &["commit", "-m", "develop work"]);
    git(&origin, &["checkout", "main"]);

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: base.map(str::to_owned),
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    (guard, root)
}

/// The new-branch case: the base a feature declares — not the repo's
/// `default_branch` — is where a new promotion's branch starts.
#[test]
fn promote_creates_the_branch_from_the_declared_base_not_the_default_branch() {
    let (_guard, root) = hall_with_two_branches(Some("develop"));
    let ctx = Ctx::new(root.clone());

    promote(&ctx, promote_input("checkout", "api")).unwrap();

    let worktree = root.join(".ivar/repos/api/checkout");
    assert!(fs::is_file(&worktree.join("develop-only.txt")).unwrap());
}

/// The base is recorded on the promotion as a plain fact, for `status`,
/// `rebase`, `prune` and `deliver` to read back.
#[test]
fn promote_records_the_declared_base_on_the_promotion() {
    let (_guard, root) = hall_with_two_branches(Some("develop"));
    let ctx = Ctx::new(root.clone());

    promote(&ctx, promote_input("checkout", "api")).unwrap();

    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("develop").unwrap())
    );
}

/// `--base` overrides the feature's declared base for this one repo.
#[test]
fn promote_base_override_wins_over_the_feature_declared_base() {
    let (_guard, root) = hall_with_two_branches(Some("main"));
    let ctx = Ctx::new(root.clone());

    promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: Some("develop".to_owned()),
        },
    )
    .unwrap();

    let worktree = root.join(".ivar/repos/api/checkout");
    assert!(fs::is_file(&worktree.join("develop-only.txt")).unwrap());
    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("develop").unwrap())
    );
}

/// A declared base this repo does not have never refuses promotion — it
/// falls back to `default_branch`, records that as the fact, and warns.
#[test]
fn promote_falls_back_and_warns_when_the_declared_base_is_absent() {
    let (_guard, root) = hall_with_two_branches(Some("ghost-branch"));
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(!report.is_clean());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "feature.base_absent")
    );
    let worktree = root.join(".ivar/repos/api/checkout");
    assert!(!fs::is_file(&worktree.join("develop-only.txt")).unwrap());
    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("main").unwrap())
    );
}

/// Adoption records the declared base as a stated fact and never probes
/// whether the adopted branch's history actually agrees with it — base is a
/// statement about the future, not a measurement of the past.
#[test]
fn promote_records_the_base_on_an_adopted_branch_without_checking_ancestry() {
    let (_guard, root) = hall_with_two_branches(Some("develop"));
    let bare = Layout::at(root.clone()).repo_bare(&RepoName::new("api").unwrap());
    // `checkout` already exists, branched off `main` — unrelated to `develop`.
    git(&bare, &["branch", "checkout", "main"]);
    let ctx = Ctx::new(root.clone());

    let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

    assert!(report.value.adopted_branch);
    assert!(report.is_clean());
    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("develop").unwrap())
    );
}

fn head_branch(worktree: &camino::Utf8Path) -> String {
    rev_parse_args(worktree, &["--abbrev-ref", "HEAD"])
}

fn rev_parse(git_dir_or_worktree: &camino::Utf8Path, rev: &str) -> String {
    rev_parse_args(git_dir_or_worktree, &[rev])
}

fn rev_parse_args(path: &camino::Utf8Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-C", path.as_str(), "rev-parse"])
        .args(args)
        .output()
        .expect("git runs");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
