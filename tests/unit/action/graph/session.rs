#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::store::layout::Layout;

fn declare_repos(root: &Utf8PathBuf, repos: &[(&str, &str)]) {
    let entries = repos
        .iter()
        .map(|(name, branch)| {
            format!(r#"{{"default_branch":"{branch}","name":"{name}","url":"https://example.com/{name}.git"}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":["claude-code"],"default":"claude-code"}},"repos":[{entries}],"version":1}}"#
        ),
    )
    .unwrap();
}

#[test]
fn test_resolve_session_view_base_when_outside_session() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();
    declare_repos(&root, &[("core", "main")]);

    let core_repo = layout.repo_worktree(
        &RepoName::new("core").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::create_dir_all(&core_repo).unwrap();

    let view = resolve_session_view(&layout, &root).unwrap();
    match view {
        SessionView::Base { repos } => {
            assert_eq!(repos.len(), 1);
            assert_eq!(repos[0].repo_name, "core");
            assert_eq!(repos[0].worktree_path, core_repo);
            assert!(!repos[0].is_layer);
        }
        _ => panic!("Expected Base session view when outside session/feature worktrees"),
    }
}

#[test]
fn test_resolve_session_view_feature_inside_worktree() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();
    declare_repos(&root, &[("core", "main")]);

    let feat_name = FeatureName::new("add-auth").unwrap();
    let feat_branch = BranchName::new("add-auth").unwrap();
    let mut feat = Feature::new(feat_name.clone(), feat_branch.clone());
    feat.promote(RepoName::new("core").unwrap());
    feat.write(&layout).unwrap();

    let core_feat_wt = layout.repo_worktree(&RepoName::new("core").unwrap(), &feat_branch);
    std::fs::create_dir_all(&core_feat_wt).unwrap();

    let view = resolve_session_view(&layout, &core_feat_wt).unwrap();
    match view {
        SessionView::FeatureSession {
            feature_name,
            repos,
            ..
        } => {
            assert_eq!(feature_name, "add-auth");
            assert_eq!(repos.len(), 1);
            assert_eq!(repos[0].repo_name, "core");
            assert_eq!(repos[0].worktree_path, core_feat_wt);
            assert!(repos[0].is_layer);
        }
        _ => panic!("Expected FeatureSession view when inside feature worktree"),
    }
}

#[test]
fn test_feature_session_view_maps_unpromoted_repo_to_declared_default_branch() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();
    declare_repos(&root, &[("core", "trunk"), ("web", "main")]);

    let feat_branch = BranchName::new("add-auth").unwrap();
    let mut feat = Feature::new(FeatureName::new("add-auth").unwrap(), feat_branch.clone());
    feat.promote(RepoName::new("web").unwrap());
    feat.write(&layout).unwrap();

    let core_trunk = layout.repo_worktree(
        &RepoName::new("core").unwrap(),
        &BranchName::new("trunk").unwrap(),
    );
    std::fs::create_dir_all(&core_trunk).unwrap();
    let web_feat_wt = layout.repo_worktree(&RepoName::new("web").unwrap(), &feat_branch);
    std::fs::create_dir_all(&web_feat_wt).unwrap();

    let view = resolve_session_view(&layout, &web_feat_wt).unwrap();
    let SessionView::FeatureSession { repos, .. } = view else {
        panic!("Expected FeatureSession view inside a feature worktree");
    };
    assert_eq!(repos[0].repo_name, "core");
    assert_eq!(repos[0].worktree_path, core_trunk);
    assert!(!repos[0].is_layer);
}

fn hall_dir() -> (tempfile::TempDir, Utf8PathBuf, Layout) {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();
    declare_repos(&root, &[("core", "main")]);
    (tmp, root, layout)
}

fn worktree(layout: &Layout, repo: &str, branch: &str) -> Utf8PathBuf {
    let path = layout.repo_worktree(
        &RepoName::new(repo).unwrap(),
        &BranchName::new(branch).unwrap(),
    );
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn feature_session_repos(layout: &Layout, feature: &Feature) -> Vec<RepoViewInfo> {
    match build_feature_session_view(layout, feature, None).unwrap() {
        SessionView::FeatureSession { repos, .. } => repos,
        other => panic!("expected a feature session view, got {other:?}"),
    }
}

#[test]
fn test_feature_session_view_layers_an_unpromoted_repo_at_the_root_base_worktree() {
    let (_tmp, _root, layout) = hall_dir();
    worktree(&layout, "core", "main");
    let develop = worktree(&layout, "core", "develop");
    let mut feat = Feature::new(
        FeatureName::new("release").unwrap(),
        BranchName::new("release").unwrap(),
    );
    feat.base = Some(BranchName::new("develop").unwrap());
    feat.write(&layout).unwrap();

    let repos = feature_session_repos(&layout, &feat);

    assert_eq!(
        repos,
        vec![RepoViewInfo {
            repo_name: "core".to_owned(),
            worktree_path: develop,
            is_layer: true,
            base_commit: None,
        }]
    );
}

#[test]
fn test_feature_session_view_keeps_the_default_view_when_the_base_worktree_is_absent() {
    let (_tmp, _root, layout) = hall_dir();
    let main = worktree(&layout, "core", "main");
    let mut feat = Feature::new(
        FeatureName::new("release").unwrap(),
        BranchName::new("release").unwrap(),
    );
    feat.base = Some(BranchName::new("develop").unwrap());
    feat.write(&layout).unwrap();

    let repos = feature_session_repos(&layout, &feat);

    assert_eq!(repos[0].worktree_path, main);
    assert!(!repos[0].is_layer);
}

#[test]
fn test_feature_session_view_layers_a_subfeature_repo_at_its_parents_worktree() {
    let (_tmp, _root, layout) = hall_dir();
    worktree(&layout, "core", "main");
    let parent_wt = worktree(&layout, "core", "add-auth");
    let mut parent = Feature::new(
        FeatureName::new("add-auth").unwrap(),
        BranchName::new("add-auth").unwrap(),
    );
    parent.promote(RepoName::new("core").unwrap());
    parent.write(&layout).unwrap();
    let mut child = Feature::new(
        FeatureName::new("add-auth-ui").unwrap(),
        BranchName::new("add-auth-ui").unwrap(),
    );
    child.parent = Some(FeatureName::new("add-auth").unwrap());
    child.base = Some(BranchName::new("add-auth").unwrap());
    child.write(&layout).unwrap();

    let repos = feature_session_repos(&layout, &child);

    assert_eq!(
        repos,
        vec![RepoViewInfo {
            repo_name: "core".to_owned(),
            worktree_path: parent_wt,
            is_layer: true,
            base_commit: None,
        }]
    );
}

/// A broken ancestor record never turns a graph query into an error: the
/// repo keeps its default-branch base view.
#[test]
fn test_feature_session_view_falls_back_to_the_default_view_when_resolution_fails() {
    let (_tmp, _root, layout) = hall_dir();
    let main = worktree(&layout, "core", "main");
    let parent_dir = layout.feature_dir(&FeatureName::new("add-auth").unwrap());
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(parent_dir.join("feature.json"), "not json").unwrap();
    let mut child = Feature::new(
        FeatureName::new("add-auth-ui").unwrap(),
        BranchName::new("add-auth-ui").unwrap(),
    );
    child.parent = Some(FeatureName::new("add-auth").unwrap());
    child.write(&layout).unwrap();

    let repos = feature_session_repos(&layout, &child);

    assert_eq!(repos[0].worktree_path, main);
    assert!(!repos[0].is_layer);
}
