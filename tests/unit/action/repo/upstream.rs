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

const UPSTREAM_URL: &str = "git@example.com:upstream/api.git";

/// A hall with one synced repo (`api`, default branch `main`).
fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
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
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    (guard, root)
}

fn input(repo: &str, url: &str) -> UpstreamInput {
    UpstreamInput {
        repo: repo.to_owned(),
        url: url.to_owned(),
        remove: false,
    }
}

/// The bare clone's recorded `upstream` URL, or `None` when the remote
/// does not exist.
fn upstream_url(root: &Utf8PathBuf) -> Option<String> {
    let bare = Layout::at(root.clone()).repo_bare(&RepoName::new("api").unwrap());
    let output = proc::capture(&git_remote(&bare, &["get-url", REMOTE])).unwrap();
    output.success().then_some(output.stdout)
}

#[test]
fn upstream_adds_the_remote_to_the_bare_clone() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    let report = upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();

    assert!(report.is_clean());
    assert!(report.value.added);
    assert_eq!(upstream_url(&root).as_deref(), Some(UPSTREAM_URL));
}

#[test]
fn upstream_repoints_an_existing_remote() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();

    let report = upstream(&ctx, input("api", "git@example.com:other/api.git")).unwrap();

    assert!(!report.value.added);
    assert_eq!(
        upstream_url(&root).as_deref(),
        Some("git@example.com:other/api.git")
    );
}

/// The invalid-upstream guard: a blank URL is refused before anything is
/// written, so the bare clone gains no `upstream` remote.
#[test]
fn upstream_refuses_a_blank_url_before_writing() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    let failure = upstream(&ctx, input("api", "   ")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.upstream_invalid_url");
    assert_eq!(upstream_url(&root), None, "nothing may be written");
}

#[test]
fn upstream_remove_drops_the_remote() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    // Set the upstream first.
    upstream(&ctx, input("api", UPSTREAM_URL)).unwrap();
    assert!(upstream_url(&root).is_some());

    // Then remove it.
    let report = upstream(
        &ctx,
        UpstreamInput {
            repo: "api".to_owned(),
            url: String::new(),
            remove: true,
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(upstream_url(&root), None, "the remote must be gone");
}

#[test]
fn upstream_remove_of_a_missing_remote_is_a_no_op() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    let report = upstream(
        &ctx,
        UpstreamInput {
            repo: "api".to_owned(),
            url: String::new(),
            remove: true,
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(upstream_url(&root), None);
}

#[test]
fn upstream_is_refused_for_a_repo_not_in_the_manifest() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let failure = upstream(&ctx, input("ghost", UPSTREAM_URL)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.upstream_repo_not_found");
}

#[test]
fn upstream_is_refused_when_the_clone_is_missing() {
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
    // Declared but never synced — no bare clone exists.

    let failure = upstream(&ctx, input("api", UPSTREAM_URL)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.upstream_bare_missing");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
    drop(guard);
}

#[test]
fn the_human_surface_names_what_happened() {
    let outcome = UpstreamOutcome {
        root: Utf8PathBuf::from("/hall"),
        repo: RepoName::new("api").unwrap(),
        remote: REMOTE.to_owned(),
        url: UPSTREAM_URL.to_owned(),
        added: true,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Added `upstream` remote for `api` in /hall ← git@example.com:upstream/api.git\n"
    );
}
