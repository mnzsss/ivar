//! Unit tests for `crate::action::feature::rename`.
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
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::session::lookup;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::Status;
use crate::git::Git;
use crate::infra::{fs, json};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::test_support::seeded_hall;

fn rename_input(feature: &str, name: Option<&str>, branch: Option<&str>) -> RenameInput {
    RenameInput {
        feature: feature.to_owned(),
        name: name.map(str::to_owned),
        branch: branch.map(str::to_owned),
    }
}

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Default)]
struct DummyRemoteGit {
    system: git::System,
    pushed: std::sync::Mutex<Vec<(String, String, String, String)>>,
    deleted: std::sync::Mutex<Vec<(String, String, String, String)>>,
}

impl git::Git for DummyRemoteGit {
    fn protect_default_branch(
        &self,
        git_dir: &Utf8Path,
        default_worktree: &Utf8Path,
        default_branch: &str,
    ) -> Result<git::protect::Protection, git::Error> {
        self.system
            .protect_default_branch(git_dir, default_worktree, default_branch)
    }
    fn target_state(&self, path: &Utf8Path) -> Result<git::TargetState, git::Error> {
        self.system.target_state(path)
    }
    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, git::Error> {
        self.system.head_branch(git_dir)
    }
    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, git::Error> {
        self.system.worktree_git_dir(path)
    }
    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), git::Error> {
        self.system.clone_bare(url, dest)
    }
    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.system.ensure_remote_tracking(git_dir)
    }
    fn add_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        branch: &str,
    ) -> Result<(), git::Error> {
        self.system.add_worktree(git_dir, dest, branch)
    }
    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.system.fetch(git_dir)
    }
    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, git::Error> {
        self.system.list_branches(git_dir)
    }
    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), git::Error> {
        self.system
            .create_branch_and_worktree(git_dir, branch, from_branch, dest)
    }
    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.system.fetch_branch(worktree, branch)
    }
    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.system.fast_forward(worktree)
    }
    fn remove_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), git::Error> {
        self.system.remove_worktree(git_dir, dest)
    }
    fn add_detached_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.system.add_detached_worktree(git_dir, dest, revision)
    }
    fn create_branch(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.system.create_branch(git_dir, branch, revision)
    }
    fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.system.delete_branch(git_dir, branch)
    }
    fn merge_no_ff(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), git::Error> {
        self.system.merge_no_ff(worktree, source, message)
    }
    fn squash_merge(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), git::Error> {
        self.system.squash_merge(worktree, source, message)
    }
    fn fast_forward_to(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.system.fast_forward_to(worktree, revision)
    }
    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, git::Error> {
        self.system.worktree_dirty(path)
    }
    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.system.diff_worktree(path)
    }
    fn commit_patch_id(&self, worktree: &Utf8Path, commit: &str) -> Result<String, git::Error> {
        self.system.commit_patch_id(worktree, commit)
    }
    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.system.changed_paths(path)
    }
    fn head_commit(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.system.head_commit(path)
    }
    fn paths_committed_since(
        &self,
        path: &Utf8Path,
        since: &str,
    ) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.system.paths_committed_since(path, since)
    }
    fn path_at_commit(
        &self,
        worktree: &Utf8Path,
        commit: &str,
        path: &Utf8Path,
    ) -> Result<Option<git::BlobEvidence>, git::Error> {
        self.system.path_at_commit(worktree, commit, path)
    }
    fn commits_ahead(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<u64, git::Error> {
        self.system.commits_ahead(git_dir, base, branch)
    }
    fn is_ancestor(
        &self,
        git_dir: &Utf8Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, git::Error> {
        self.system.is_ancestor(git_dir, ancestor, descendant)
    }
    fn merge_base(&self, git_dir: &Utf8Path, a: &str, b: &str) -> Result<String, git::Error> {
        self.system.merge_base(git_dir, a, b)
    }
    fn divergence(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<git::Divergence, git::Error> {
        self.system.divergence(git_dir, base, branch)
    }
    fn diff_patch_id(
        &self,
        worktree: &Utf8Path,
        base: &str,
        tip: &str,
    ) -> Result<String, git::Error> {
        self.system.diff_patch_id(worktree, base, tip)
    }
    fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.system.reset_hard(worktree, revision)
    }
    fn revision_commit(&self, git_dir: &Utf8Path, revision: &str) -> Result<String, git::Error> {
        self.system.revision_commit(git_dir, revision)
    }
    fn remote_branch_tip(
        &self,
        _git_dir: &Utf8Path,
        _remote: &str,
        branch: &str,
    ) -> Result<Option<String>, git::Error> {
        if branch == "new-branch" {
            Ok(Some("new-tip-sha".to_owned()))
        } else {
            Ok(None)
        }
    }
    fn push(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        from: &str,
        to: &str,
    ) -> Result<(), git::Error> {
        self.pushed.lock().unwrap().push((
            git_dir.to_string(),
            remote.to_owned(),
            from.to_owned(),
            to.to_owned(),
        ));
        Ok(())
    }
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.system.rebase_branch(worktree, branch)
    }
    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.system.abort_rebase(worktree)
    }
    fn rename_branch(&self, git_dir: &Utf8Path, from: &str, to: &str) -> Result<(), git::Error> {
        self.system.rename_branch(git_dir, from, to)
    }
    fn move_worktree(
        &self,
        git_dir: &Utf8Path,
        from: &Utf8Path,
        to: &Utf8Path,
    ) -> Result<(), git::Error> {
        self.system.move_worktree(git_dir, from, to)
    }
    fn publish_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        at: &str,
    ) -> Result<(), git::Error> {
        self.system
            .publish_remote_branch(git_dir, remote, branch, at)
    }
    fn delete_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        expected_tip: &str,
    ) -> Result<(), git::Error> {
        self.deleted.lock().unwrap().push((
            git_dir.to_string(),
            remote.to_owned(),
            branch.to_owned(),
            expected_tip.to_owned(),
        ));
        Ok(())
    }
}

struct TestHallContext {
    _guard: tempfile::TempDir,
    layout: Layout,
    manifest: Manifest,
    old_name: FeatureName,
    new_name: FeatureName,
    old_branch: BranchName,
    new_branch: BranchName,
}

fn setup_test_hall(
    old_name_str: &str,
    new_name_str: &str,
    new_branch_str: &str,
) -> TestHallContext {
    let (guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    let root_path = Utf8Path::from_path(guard.path()).unwrap();
    let origin = crate::test_support::seeded_repo(&root_path.join("origins").join("api"), "main");
    let manifest = Manifest::new(
        crate::domain::name::HallName::new("acme").unwrap(),
        crate::store::manifest::Providers::new(
            vec![crate::domain::provider::Provider::ClaudeCode],
            crate::domain::provider::Provider::ClaudeCode,
        ),
        vec![crate::store::manifest::Repo::new(
            crate::domain::name::RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: old_name_str.to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "child-feat".to_owned(),
            branch: None,
            base: None,
            parent: Some(old_name_str.to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let _session = crate::action::session::start::start(
        &ctx,
        crate::action::session::start::StartInput {
            feature: Some(old_name_str.to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
            prompt: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new(old_name_str).unwrap();
    let new_name = FeatureName::new(new_name_str).unwrap();
    let old_feature = Feature::read(&layout, &old_name).unwrap().unwrap();
    let old_branch = old_feature.branch.clone();
    let new_branch = BranchName::new(new_branch_str).unwrap();

    TestHallContext {
        _guard: guard,
        layout,
        manifest,
        old_name,
        new_name,
        old_branch,
        new_branch,
    }
}

fn build_test_plan(tc: &TestHallContext, git: &impl git::Git) -> super::plan::RenamePlan {
    let source = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
    let (mut plan, blockers) = plan::build(
        &tc.layout,
        &tc.manifest,
        git,
        &source,
        tc.new_name.clone(),
        Some(&tc.new_branch.clone()),
    )
    .unwrap();
    assert!(blockers.is_empty());

    let repo_name = crate::domain::name::RepoName::new("api").unwrap();
    let old_worktree = tc.layout.repo_worktree(&repo_name, &tc.old_branch);
    let new_worktree = tc.layout.repo_worktree(&repo_name, &tc.new_branch);
    let bare = tc.layout.repo_bare(&repo_name);

    if git.revision_commit(&bare, tc.old_branch.as_str()).is_err() {
        let _ = git.create_branch(&bare, tc.old_branch.as_str(), "main");
    }
    if !fs::is_dir(&old_worktree).unwrap_or(false) {
        let _ = git.add_worktree(&bare, &old_worktree, tc.old_branch.as_str());
    }

    plan.repos.push(super::plan::RepoRenamePlan {
        repo: repo_name,
        old_worktree,
        new_worktree,
        old_branch: tc.old_branch.clone(),
        new_branch: tc.new_branch.clone(),
        old_remote: None,
        old_remote_tip: None,
    });

    plan
}

#[path = "rename/forward_failure.rs"]
mod forward_failure;
#[path = "rename/outcomes.rs"]
mod outcomes;
#[path = "rename/preflight.rs"]
mod preflight;
#[path = "rename/remote_race.rs"]
mod remote_race;
#[path = "rename/resume.rs"]
mod resume;
