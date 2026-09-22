#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall declaring `repos` as `(name, default_branch)`. Nothing is
/// materialised: call `sync` to get bare clones and worktrees.
fn hall_declaring(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origins = root.parent().unwrap().join("origins");
    let declared: Vec<Repo> = repos
        .iter()
        .map(|(name, branch)| {
            let origin = seeded_repo(&origins.join(name), branch);
            Repo::new(
                RepoName::new(*name).unwrap(),
                origin.as_str(),
                BranchName::new(*branch).unwrap(),
            )
        })
        .collect();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        declared,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    (guard, root)
}

#[test]
fn view_opens_every_declared_repo_on_its_default_branch() {
    let (_guard, root) = hall_declaring(&[("api", "main"), ("web", "trunk")]);
    let ctx = Ctx::new(root);
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let report = view(&ctx, ViewInput { repos: Vec::new() }).unwrap();

    let repos = &report.value.repos;
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].name.as_str(), "api");
    assert_eq!(repos[0].default_branch, "main");
    assert!(repos[0].worktree.as_str().ends_with(".ivar/repos/api/main"));
    assert!(repos[0].openable);
    assert_eq!(repos[0].reason, None);
    assert_eq!(repos[1].name.as_str(), "web");
    assert!(repos[1].worktree.as_str().ends_with(".ivar/repos/web/trunk"));
    assert!(repos[1].openable);
}

#[test]
fn a_repo_without_a_worktree_is_listed_unopenable_and_says_what_to_run() {
    // Both are declared and synced; then `web`'s worktree is removed —
    // the state a manual delete or a half-finished sync leaves behind.
    let (_guard, root) = hall_declaring(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let layout = Layout::at(root.clone());
    let worktree = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::remove_path(&worktree).unwrap();

    let report = view(&ctx, ViewInput { repos: Vec::new() }).unwrap();

    let web = report
        .value
        .repos
        .iter()
        .find(|repo| repo.name.as_str() == "web")
        .expect("a declared repo is listed even when it is not materialised");
    assert!(!web.openable);
    let reason = web.reason.as_deref().expect("an unopenable repo carries its reason");
    assert!(
        reason.contains("ivar sync"),
        "the reason names the verb that materialises a worktree: {reason}"
    );
    assert!(
        !crate::infra::fs::is_dir(&worktree).unwrap(),
        "R-NO-WORKTREE-CREATE: listing an unopenable repo must not materialise it"
    );
}

#[test]
fn view_is_blocked_when_nothing_is_openable() {
    let (_guard, root) = hall_declaring(&[("api", "main")]);
    let ctx = Ctx::new(root);

    let failure = view(&ctx, ViewInput { repos: Vec::new() }).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.view_nothing_openable");
}

#[test]
fn view_is_blocked_for_a_repo_that_is_not_declared() {
    let (_guard, root) = hall_declaring(&[("api", "main")]);
    let ctx = Ctx::new(root);
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let failure = view(
        &ctx,
        ViewInput {
            repos: vec!["ghost".to_owned()],
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.not_declared");
}

#[test]
fn a_named_subset_opens_only_those_repos() {
    let (_guard, root) = hall_declaring(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root);
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let report = view(
        &ctx,
        ViewInput {
            repos: vec!["web".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(report.value.repos.len(), 1);
    assert_eq!(report.value.repos[0].name.as_str(), "web");
}

#[test]
fn the_human_surface_names_each_repo_its_branch_and_why_it_cannot_open() {
    let outcome = ViewOutcome {
        root: Utf8PathBuf::from("/halls/acme"),
        repos: vec![
            RepoView {
                name: RepoName::new("api").unwrap(),
                default_branch: "main".to_owned(),
                worktree: Utf8PathBuf::from("/halls/acme/.ivar/repos/api/main"),
                openable: true,
                reason: None,
            },
            RepoView {
                name: RepoName::new("web").unwrap(),
                default_branch: "main".to_owned(),
                worktree: Utf8PathBuf::from("/halls/acme/.ivar/repos/web/main"),
                openable: false,
                reason: Some("no main worktree; run `ivar sync`".to_owned()),
            },
        ],
    };

    let mut rendered = Vec::new();
    outcome.write_human(&mut rendered).unwrap();
    let rendered = String::from_utf8(rendered).unwrap();

    assert!(rendered.contains("api  main  /halls/acme/.ivar/repos/api/main"));
    assert!(rendered.contains("no main worktree; run `ivar sync`"));
    assert!(rendered.contains("1 shell opened"));
}
