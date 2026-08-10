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
use crate::store::manifest::{Manifest, Providers};
use crate::test_support::{hall_root, seeded_repo};

fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
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

    if !repos.is_empty() {
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
    }

    (guard, root)
}

#[test]
fn list_reports_an_empty_hall_as_empty() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.repos.is_empty());
}

#[test]
fn list_reports_a_declared_repo_before_any_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    let repo = &report.value.repos[0];
    assert_eq!(repo.name.as_str(), "api");
    assert_eq!(repo.default_branch, "main");
    assert!(!repo.bare_cloned, "not synced yet");
    assert!(repo.branches.is_empty());
}

#[test]
fn list_reports_a_synced_repo_with_its_branches() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = list(&ctx).unwrap();

    let repo = &report.value.repos[0];
    assert!(repo.bare_cloned);
    assert!(repo.default_worktree);
    assert!(repo.branches.contains(&"main".to_owned()));
}

#[test]
fn list_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = list(&ctx).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn the_human_surface_lists_repos_with_their_state() {
    let outcome = ListOutcome {
        root: Utf8PathBuf::from("/hall"),
        repos: vec![RepoStatus {
            name: RepoName::new("api").unwrap(),
            url: "git@example.com:acme/api.git".to_owned(),
            default_branch: "main".to_owned(),
            bare_cloned: true,
            default_worktree: true,
            branches: vec!["dev".to_owned(), "main".to_owned()],
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Repos in /hall:\n  api  cloned  main  ← git@example.com:acme/api.git  [dev, main]\n"
    );
}
