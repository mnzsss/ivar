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

#[test]
fn test_resolve_session_view_base_when_outside_session() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();

    let core_repo = layout.repo_worktree(&RepoName::new("core").unwrap(), &BranchName::new("main").unwrap());
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

    let feat_name = FeatureName::new("add-auth").unwrap();
    let feat_branch = BranchName::new("add-auth").unwrap();
    let mut feat = Feature::new(feat_name.clone(), feat_branch.clone());
    feat.promote(RepoName::new("core").unwrap());
    feat.write(&layout).unwrap();

    let core_feat_wt = layout.repo_worktree(&RepoName::new("core").unwrap(), &feat_branch);
    std::fs::create_dir_all(&core_feat_wt).unwrap();

    let view = resolve_session_view(&layout, &core_feat_wt).unwrap();
    match view {
        SessionView::FeatureSession { feature_name, repos, .. } => {
            assert_eq!(feature_name, "add-auth");
            assert_eq!(repos.len(), 1);
            assert_eq!(repos[0].repo_name, "core");
            assert_eq!(repos[0].worktree_path, core_feat_wt);
            assert!(repos[0].is_layer);
        }
        _ => panic!("Expected FeatureSession view when inside feature worktree"),
    }
}
