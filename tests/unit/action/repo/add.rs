#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
use crate::store::layout::Layout;
use crate::test_support::{hall_root, seeded_repo};

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf, String) {
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
    (guard, root, origin.as_str().to_owned())
}

fn input(url: &str) -> AddInput {
    AddInput {
        name: "api".to_owned(),
        url: url.to_owned(),
        default_branch: None,
        reuse_existing: None,
    }
}

#[test]
fn add_clones_the_repo_and_declares_it_in_ivar_json() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, input(&url)).unwrap();

    assert!(report.is_clean());
    assert!(!report.value.bare_clone_reused);
    assert_eq!(report.value.next_action, "/ivar-relations api");
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/main/README.md")).unwrap(),
        "seed\n"
    );

    let layout = Layout::at(root);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(manifest.repos().len(), 1);
    assert_eq!(manifest.repos()[0].name().as_str(), "api");
}

/// The invitation is part of the successful outcome — rendered by the same
/// value, never a second independently computed string — and the report
/// carries no warning or fix action. The manifest schema stays version 1.
#[test]
fn the_next_action_is_shared_by_json_and_human_surfaces() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, input(&url)).unwrap();

    assert!(report.is_clean());
    let json = serde_json::to_value(Report::new(report.value.clone())).unwrap();
    assert_eq!(json["next_action"], "/ivar-relations api");

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let human = String::from_utf8(out).unwrap();
    assert!(
        human.contains("Next: run `/ivar-relations api`"),
        "was: {human}"
    );

    let layout = Layout::at(root);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(manifest.version(), 1);
}

#[test]
fn add_is_idempotent_on_the_worktree_path_but_rejects_a_duplicate_name() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();

    let error = add(&ctx, input(&url)).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "repo.name_exists");
}

#[test]
fn add_rejects_a_url_already_tracked_under_another_name() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();

    let error = add(
        &ctx,
        AddInput {
            name: "api-2".to_owned(),
            ..input(&url)
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "repo.url_exists");
    assert!(
        error.what.contains("api"),
        "the existing name must be named: {}",
        error.what
    );
}

/// The bare-exists collision fires when the name and URL are both free
/// but a previous `add` (since removed from the manifest) left a clone
/// on disk. The fix actions must name both ways out.
#[test]
fn add_blocks_when_a_bare_clone_exists_without_reuse_or_fresh() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();
    // Remove the manifest entry; the bare clone stays on disk.
    let layout = Layout::at(root.clone());
    let mut manifest = Manifest::read(&layout).unwrap().unwrap();
    manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let error = add(&ctx, input(&url)).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "repo.bare_exists");
    assert!(
        error
            .fix_actions
            .iter()
            .any(|fix| fix.code == "repo.reuse" && fix.safe)
    );
    assert!(
        error
            .fix_actions
            .iter()
            .any(|fix| fix.code == "repo.fresh" && !fix.safe)
    );
}

#[test]
fn reuse_keeps_the_existing_bare_clone() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();
    let layout = Layout::at(root.clone());
    let mut manifest = Manifest::read(&layout).unwrap().unwrap();
    manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = add(
        &ctx,
        AddInput {
            name: "api".to_owned(),
            url,
            default_branch: None,
            reuse_existing: Some(true),
        },
    )
    .unwrap();

    assert!(report.value.bare_clone_reused);
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
}

#[test]
fn fresh_replaces_the_existing_bare_clone() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();
    let layout = Layout::at(root.clone());
    let mut manifest = Manifest::read(&layout).unwrap().unwrap();
    manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = add(
        &ctx,
        AddInput {
            name: "api".to_owned(),
            url,
            default_branch: None,
            reuse_existing: Some(false),
        },
    )
    .unwrap();

    assert!(!report.value.bare_clone_reused);
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
}

#[test]
fn add_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = add(&ctx, input("git@example.com:acme/api.git")).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn add_rejects_an_invalid_name() {
    let (_guard, root) = hall_root();
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

    let failure = add(
        &ctx,
        AddInput {
            name: "../etc".to_owned(),
            url: "git@example.com:acme/api.git".to_owned(),
            default_branch: None,
            reuse_existing: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "name.not_a_segment");
}

#[test]
fn the_human_surface_names_the_repo_and_whether_it_was_reused() {
    let outcome = AddOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: RepoName::new("api").unwrap(),
        url: "git@example.com:acme/api.git".to_owned(),
        default_branch: BranchName::new("main").unwrap(),
        bare_clone_reused: true,
        next_action: "/ivar-relations api".to_owned(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Added repo `api` at /hall ← git@example.com:acme/api.git (reused existing clone)\n\
         Next: run `/ivar-relations api`\n"
    );
}

/// A bare adopted with `--reuse` may predate the remote-tracking refspec, or
/// have been cloned by hand without one. `add` normalises it either way — the
/// worktree it hands back has to support a `--force-with-lease` like any other.
#[test]
fn reuse_configures_the_remote_tracking_refspec_on_the_adopted_bare() {
    let (_guard, root, url) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, input(&url)).unwrap();
    let layout = Layout::at(root.clone());
    let bare = root.join(".ivar/repos/api/.bare");
    // Put the bare back the way a build without the refspec left it.
    crate::test_support::git(&bare, &["config", "--unset", "remote.origin.fetch"]);
    let mut manifest = Manifest::read(&layout).unwrap().unwrap();
    manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = add(
        &ctx,
        AddInput {
            name: "api".to_owned(),
            url,
            default_branch: None,
            reuse_existing: Some(true),
        },
    )
    .unwrap();

    assert!(report.value.bare_clone_reused);
    let configured = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str()])
        .args(["config", "--get", "remote.origin.fetch"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&configured.stdout).trim(),
        "+refs/heads/*:refs/remotes/origin/*"
    );
}
