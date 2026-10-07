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

use super::*;
use crate::domain::name::RepoName;
use crate::test_support::hall_root;

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
