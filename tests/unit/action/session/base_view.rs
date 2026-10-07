//! Unit tests for `crate::action::session::base_view`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;

use super::*;
use crate::action::Ctx;
use crate::action::hall::{self, InitInput};
use crate::domain::name::{HallName, RepoName};
use crate::domain::provider::Provider;
use crate::git::System;
use crate::store::manifest::Providers;
use crate::test_support::{git, hall_root, seeded_repo};

/// A scratch hall layout: only `.ivar/features/*` and `.ivar/repos/*`
/// directories are ever created under it by these resolver tests.
fn scratch() -> (tempfile::TempDir, Layout) {
    let (guard, root) = hall_root();
    (guard, Layout::at(root))
}

/// The one repo every resolver test asks about: `api`, default `main`.
fn api() -> Repo {
    Repo::new(
        RepoName::new("api").unwrap(),
        "unused-origin",
        BranchName::new("main").unwrap(),
    )
}

/// Write a feature record `name` on branch `name`, with an optional
/// explicit `base` and `parent`, promoting `api` when `promotes_api`.
fn record(
    layout: &Layout,
    name: &str,
    base: Option<&str>,
    parent: Option<&str>,
    promotes_api: bool,
) -> Feature {
    let mut feature = Feature::new(
        FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    feature.base = base.map(|b| BranchName::new(b).unwrap());
    feature.parent = parent.map(|p| FeatureName::new(p).unwrap());
    if promotes_api {
        feature.promote(RepoName::new("api").unwrap());
    }
    feature.write(layout).unwrap();
    feature
}

/// Put a worktree directory for `api` on `branch` on disk.
fn on_disk(layout: &Layout, branch: &str) -> Utf8PathBuf {
    let path = layout.repo_worktree(api().name(), &BranchName::new(branch).unwrap());
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn a_root_without_a_base_views_the_default_branch() {
    let (_guard, layout) = scratch();
    let root = record(&layout, "checkout", None, None, false);

    assert_eq!(resolve(&layout, &api(), &root).unwrap(), BaseView::Default);
}

#[test]
fn a_root_whose_base_is_the_default_branch_views_the_default_branch() {
    let (_guard, layout) = scratch();
    let root = record(&layout, "checkout", Some("main"), None, false);

    assert_eq!(resolve(&layout, &api(), &root).unwrap(), BaseView::Default);
}

#[test]
fn a_root_with_an_explicit_base_views_that_base_branch() {
    let (_guard, layout) = scratch();
    let root = record(&layout, "checkout", Some("develop"), None, false);

    assert_eq!(
        resolve(&layout, &api(), &root).unwrap(),
        BaseView::BaseBranch(BranchName::new("develop").unwrap())
    );
}

#[test]
fn a_child_views_the_worktree_of_a_parent_that_promotes_the_repo() {
    let (_guard, layout) = scratch();
    record(&layout, "parent", Some("develop"), None, true);
    on_disk(&layout, "parent");
    let child = record(&layout, "child", Some("parent"), Some("parent"), false);

    let view = resolve(&layout, &api(), &child).unwrap();

    assert_eq!(
        view,
        BaseView::Parent {
            feature: FeatureName::new("parent").unwrap(),
            branch: BranchName::new("parent").unwrap(),
        }
    );
    assert_eq!(view.worktree(&layout, &api()), on_disk(&layout, "parent"));
    assert!(
        !view.guards_read_only(),
        "a parent worktree's bits are never touched"
    );
}

/// A promoted parent whose worktree is not on disk cannot be viewed; the
/// walk continues to the root's rule.
#[test]
fn a_parent_promotion_without_a_worktree_on_disk_is_skipped() {
    let (_guard, layout) = scratch();
    record(&layout, "parent", Some("develop"), None, true);
    let child = record(&layout, "child", Some("parent"), Some("parent"), false);

    assert_eq!(
        resolve(&layout, &api(), &child).unwrap(),
        BaseView::BaseBranch(BranchName::new("develop").unwrap())
    );
}

/// The child's own `base` is its parent's branch; only the root's explicit
/// base is ever a base-branch view.
#[test]
fn a_grandchild_falls_through_an_unpromoting_parent_to_the_roots_base() {
    let (_guard, layout) = scratch();
    record(&layout, "root", Some("develop"), None, false);
    record(&layout, "mid", Some("root"), Some("root"), false);
    let leaf = record(&layout, "leaf", Some("mid"), Some("mid"), false);

    assert_eq!(
        resolve(&layout, &api(), &leaf).unwrap(),
        BaseView::BaseBranch(BranchName::new("develop").unwrap())
    );
}

#[test]
fn a_grandchild_views_the_nearest_promoting_ancestor() {
    let (_guard, layout) = scratch();
    record(&layout, "root", Some("develop"), None, true);
    on_disk(&layout, "root");
    record(&layout, "mid", Some("root"), Some("root"), false);
    let leaf = record(&layout, "leaf", Some("mid"), Some("mid"), false);

    assert_eq!(
        resolve(&layout, &api(), &leaf).unwrap(),
        BaseView::Parent {
            feature: FeatureName::new("root").unwrap(),
            branch: BranchName::new("root").unwrap(),
        }
    );
}

#[test]
fn a_missing_parent_record_views_the_default_branch() {
    let (_guard, layout) = scratch();
    let orphan = record(&layout, "orphan", Some("gone"), Some("gone"), false);

    assert_eq!(
        resolve(&layout, &api(), &orphan).unwrap(),
        BaseView::Default
    );
}

#[test]
fn a_parent_cycle_views_the_default_branch() {
    let (_guard, layout) = scratch();
    record(&layout, "a", Some("develop"), Some("b"), false);
    let b = record(&layout, "b", Some("develop"), Some("a"), false);

    assert_eq!(resolve(&layout, &api(), &b).unwrap(), BaseView::Default);
}

#[test]
fn base_and_default_views_are_read_only_guarded_at_their_worktree() {
    let (_guard, layout) = scratch();
    let develop = BaseView::BaseBranch(BranchName::new("develop").unwrap());

    assert!(develop.guards_read_only());
    assert!(BaseView::Default.guards_read_only());
    assert_eq!(
        develop.worktree(&layout, &api()),
        layout.root().join(".ivar/repos/api/develop")
    );
    assert_eq!(
        BaseView::Default.worktree(&layout, &api()),
        layout.root().join(".ivar/repos/api/main")
    );
}

#[test]
fn resolve_on_disk_keeps_a_base_branch_whose_worktree_exists() {
    let (_guard, layout) = scratch();
    let develop = on_disk(&layout, "develop");
    let root = record(&layout, "checkout", Some("develop"), None, false);

    assert_eq!(
        resolve_on_disk(&layout, &api(), &root).unwrap(),
        (
            BaseView::BaseBranch(BranchName::new("develop").unwrap()),
            develop
        )
    );
}

#[test]
fn resolve_on_disk_falls_back_to_the_default_branch_without_a_base_worktree() {
    let (_guard, layout) = scratch();
    let root = record(&layout, "checkout", Some("develop"), None, false);

    assert_eq!(
        resolve_on_disk(&layout, &api(), &root).unwrap(),
        (
            BaseView::Default,
            layout.root().join(".ivar/repos/api/main")
        )
    );
}

/// A synced hall declaring `api` (default `main`). With `develop_at_sync`,
/// the origin already has `develop` (at the seed commit) when the bare is
/// cloned, so the bare holds a local — soon stale — `develop`.
fn synced_hall(develop_at_sync: bool) -> (tempfile::TempDir, Layout, Utf8PathBuf) {
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
    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    if develop_at_sync {
        git(&origin, &["branch", "develop"]);
    }
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
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    (guard, layout, origin)
}

/// Commit `BASE.md` = `content` on the origin's `develop` (creating the
/// branch if needed) and return the new tip; the origin is left on `main`.
fn advance_develop(origin: &Utf8Path, content: &str) -> String {
    if System
        .list_branches(origin)
        .unwrap()
        .iter()
        .any(|b| b == "develop")
    {
        git(origin, &["checkout", "develop"]);
    } else {
        git(origin, &["checkout", "-b", "develop"]);
    }
    std::fs::write(origin.join("BASE.md"), content).unwrap();
    git(origin, &["add", "BASE.md"]);
    git(origin, &["commit", "-m", content.trim()]);
    let tip = System.head_commit(origin).unwrap();
    git(origin, &["checkout", "main"]);
    tip
}

fn manifest_of(layout: &Layout) -> Manifest {
    Manifest::read(layout).unwrap().unwrap()
}

fn codes(warnings: &[Warning]) -> Vec<&'static str> {
    warnings.iter().map(|warning| warning.code).collect()
}

fn develop_worktree(layout: &Layout) -> Utf8PathBuf {
    layout.root().join(".ivar/repos/api/develop")
}

/// The bare cloned `develop` at the seed commit; the origin moved on since.
/// The new base worktree starts from the origin's tip, not the stale copy.
#[test]
fn prepare_creates_the_base_worktree_at_the_origins_tip() {
    let (_guard, layout, origin) = synced_hall(true);
    let tip = advance_develop(&origin, "v1\n");
    let root = record(&layout, "checkout", Some("develop"), None, false);

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert!(warnings.is_empty(), "{warnings:?}");
    let base = develop_worktree(&layout);
    assert_eq!(System.head_branch(&base).unwrap(), "develop");
    assert_eq!(System.head_commit(&base).unwrap(), tip);
    assert_eq!(
        std::fs::read_to_string(base.join("BASE.md")).unwrap(),
        "v1\n"
    );
}

/// A base branch the origin gained after the hall was synced exists only as
/// `refs/remotes/origin/develop` once fetched; the worktree tracks it.
#[test]
fn prepare_creates_a_base_worktree_for_a_branch_the_origin_gained_after_sync() {
    let (_guard, layout, origin) = synced_hall(false);
    let tip = advance_develop(&origin, "v1\n");
    let root = record(&layout, "checkout", Some("develop"), None, false);

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert!(warnings.is_empty(), "{warnings:?}");
    let base = develop_worktree(&layout);
    assert_eq!(System.head_branch(&base).unwrap(), "develop");
    assert_eq!(System.head_commit(&base).unwrap(), tip);
}

#[test]
fn prepare_fast_forwards_an_existing_base_worktree_through_its_guard() {
    let (_guard, layout, origin) = synced_hall(true);
    let root = record(&layout, "checkout", Some("develop"), None, false);
    assert!(prepare(&System, &layout, &manifest_of(&layout), &root).is_empty());
    let base = develop_worktree(&layout);
    fs::clear_write_bits(&base).unwrap();
    let tip = advance_develop(&origin, "v2\n");

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(System.head_commit(&base).unwrap(), tip);
    assert_eq!(
        std::fs::read_to_string(base.join("BASE.md")).unwrap(),
        "v2\n"
    );
    assert_eq!(
        fs::unix_mode(&base).unwrap().unwrap() & 0o222,
        0,
        "the read-only guard must be re-applied after the refresh"
    );
    fs::restore_write_bits(&base).unwrap();
}

#[test]
fn prepare_warns_once_when_the_base_branch_does_not_exist() {
    let (_guard, layout, _origin) = synced_hall(true);
    let root = record(&layout, "checkout", Some("release"), None, false);

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert_eq!(codes(&warnings), vec!["session.base_absent"]);
    assert_eq!(warnings[0].subject, "api");
    assert_eq!(
        warnings[0].what,
        "base `release` not found in `api`; using `main`"
    );
    assert!(!layout.root().join(".ivar/repos/api/release").exists());
}

/// Only an unpromoted repo viewed at an explicit base branch is prepared:
/// a promoted repo, a default view and a parent view create nothing, and a
/// parent worktree's mode is never touched.
#[test]
fn prepare_leaves_promoted_default_and_parent_views_alone() {
    let (_guard, layout, _origin) = synced_hall(true);
    let promoted = record(&layout, "promoted", Some("develop"), None, true);
    let plain = record(&layout, "plain", None, None, false);
    record(&layout, "parent", Some("develop"), None, true);
    let parent_worktree = on_disk(&layout, "parent");
    let parent_mode = fs::unix_mode(&parent_worktree).unwrap();
    let child = record(&layout, "child", Some("parent"), Some("parent"), false);

    for feature in [&promoted, &plain, &child] {
        let warnings = prepare(&System, &layout, &manifest_of(&layout), feature);
        assert!(warnings.is_empty(), "{}: {warnings:?}", feature.name);
    }

    assert!(!develop_worktree(&layout).exists());
    assert_eq!(fs::unix_mode(&parent_worktree).unwrap(), parent_mode);
}

#[test]
fn prepare_only_warns_when_the_remote_is_unreachable_for_an_existing_base() {
    let (_guard, layout, _origin) = synced_hall(true);
    let root = record(&layout, "checkout", Some("develop"), None, false);
    assert!(prepare(&System, &layout, &manifest_of(&layout), &root).is_empty());
    git(
        &layout.repo_bare(api().name()),
        &["remote", "set-url", "origin", "/nonexistent/origin"],
    );

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert_eq!(codes(&warnings), vec!["session.base_refresh_failed"]);
    assert_eq!(warnings[0].subject, "api");
    assert!(
        warnings[0].what.contains("`develop`"),
        "{}",
        warnings[0].what
    );
    assert!(develop_worktree(&layout).is_dir());
}

/// Offline, a base branch the bare already holds is still checked out —
/// from the local copy — with one warning for the failed fetch.
#[test]
fn prepare_creates_the_base_worktree_offline_from_the_local_branch() {
    let (_guard, layout, _origin) = synced_hall(true);
    git(
        &layout.repo_bare(api().name()),
        &["remote", "set-url", "origin", "/nonexistent/origin"],
    );
    let root = record(&layout, "checkout", Some("develop"), None, false);

    let warnings = prepare(&System, &layout, &manifest_of(&layout), &root);

    assert_eq!(codes(&warnings), vec!["session.base_refresh_failed"]);
    assert_eq!(
        System.head_branch(&develop_worktree(&layout)).unwrap(),
        "develop"
    );
}
