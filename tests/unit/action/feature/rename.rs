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
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), git::Error> {
        self.system.merge_no_ff(worktree, source)
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

#[test]
fn rename_refuses_when_name_and_branch_unchanged() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "my-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let err = rename(&ctx, rename_input("my-feat", Some("my-feat"), None)).unwrap_err();
    assert_eq!(err.status, Status::Blocked);
    assert_eq!(err.code, "feature.rename_noop");
}

#[test]
fn rename_feature_name_success() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "old-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let report = rename(&ctx, rename_input("old-feat", Some("new-feat"), None)).unwrap();
    assert!(report.is_clean());
    let outcome = report.value;
    assert_eq!(outcome.old_name.as_str(), "old-feat");
    assert_eq!(outcome.new_name.as_str(), "new-feat");
}

#[test]
fn rename_record_writing_and_preservation() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "record-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("record-old").unwrap();
    let new_name = FeatureName::new("record-new").unwrap();
    let new_branch = BranchName::new("feat/record-new-branch").unwrap();

    let report = rename(
        &ctx,
        rename_input(
            "record-old",
            Some("record-new"),
            Some("feat/record-new-branch"),
        ),
    )
    .unwrap();
    assert!(report.is_clean());

    assert!(Feature::read(&layout, &old_name).unwrap().is_none());

    let updated = Feature::read(&layout, &new_name).unwrap().unwrap();
    assert_eq!(updated.name, new_name);
    assert_eq!(updated.branch, new_branch);
}

#[test]
fn rename_updates_child_parent_reference() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "parent-old".to_owned(),
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
            parent: Some("parent-old".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let child_name = FeatureName::new("child-feat").unwrap();
    let child_before = Feature::read(&layout, &child_name).unwrap().unwrap();
    assert_eq!(child_before.parent.as_ref().unwrap().as_str(), "parent-old");

    rename(&ctx, rename_input("parent-old", Some("parent-new"), None)).unwrap();

    let child_after = Feature::read(&layout, &child_name).unwrap().unwrap();
    assert_eq!(child_after.parent.as_ref().unwrap().as_str(), "parent-new");
}

#[test]
fn rename_updates_and_rematerialises_session() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "sess-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let session_report = crate::action::session::start::start(
        &ctx,
        crate::action::session::start::StartInput {
            feature: Some("sess-old".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    let _outcome = session_report.value;
    let old_name = FeatureName::new("sess-old").unwrap();
    let new_name = FeatureName::new("sess-new").unwrap();

    let initial_sessions = lookup::list_feature(&layout, &old_name).unwrap();
    assert_eq!(initial_sessions.len(), 1);

    rename(&ctx, rename_input("sess-old", Some("sess-new"), None)).unwrap();

    let updated_sessions = lookup::list_feature(&layout, &new_name).unwrap();
    assert_eq!(updated_sessions.len(), 1);
    let s_ref = &updated_sessions[0];
    assert_eq!(s_ref.state.as_ref().unwrap().feature, Some(new_name));
    assert!(fs::is_dir(&s_ref.view_dir).unwrap());
}

#[test]
fn find_transition_discovers_marker_in_new_path() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "trans-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("trans-old").unwrap();
    let new_name = FeatureName::new("trans-new").unwrap();

    let source = Feature::read(&layout, &old_name).unwrap().unwrap();
    let plan = plan::build(
        &layout,
        &read_manifest(&layout).unwrap(),
        &git::System,
        &source,
        new_name.clone(),
        None,
    )
    .unwrap()
    .0;

    let new_dir = layout.feature_dir(&new_name);
    fs::ensure_dir(&new_dir).unwrap();
    let marker_path = new_dir.join(".renaming");
    let transition = steps::Transition {
        version: 1,
        plan,
        direction: steps::Direction::Forward,
        step: steps::Step::MoveFeatureDir,
    };
    json::write_canonical(&marker_path, &transition).unwrap();

    let found_old = steps::find_transition(&layout, &old_name).unwrap();
    assert!(found_old.is_some());
    let (found_path, found_t) = found_old.unwrap();
    assert_eq!(found_path, marker_path);
    assert_eq!(found_t.plan.new_name, new_name);

    let found_new = steps::find_transition(&layout, &new_name).unwrap();
    assert!(found_new.is_some());
    assert_eq!(found_new.unwrap().0, marker_path);
}

#[test]
fn rename_name_only_and_branch_only_behaviour() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "feat-a".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    // Name-only rename
    let outcome = rename(&ctx, rename_input("feat-a", Some("feat-b"), None))
        .unwrap()
        .value;
    assert_eq!(outcome.old_name.as_str(), "feat-a");
    assert_eq!(outcome.new_name.as_str(), "feat-b");
    assert_eq!(outcome.old_branch, outcome.new_branch);
    assert!(fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-b").unwrap())).unwrap());
    assert!(!fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-a").unwrap())).unwrap());

    // Branch-only rename
    let outcome2 = rename(
        &ctx,
        rename_input("feat-b", None, Some("feat/new-branch-name")),
    )
    .unwrap()
    .value;
    assert_eq!(outcome2.old_name.as_str(), "feat-b");
    assert_eq!(outcome2.new_name.as_str(), "feat-b");
    assert_ne!(outcome2.old_branch, outcome2.new_branch);
    assert_eq!(outcome2.new_branch.as_str(), "feat/new-branch-name");
    assert!(fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-b").unwrap())).unwrap());
}

#[test]
fn rename_remote_rollback() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let manifest = read_manifest(&layout).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "remote-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("remote-feat").unwrap();
    let source = Feature::read(&layout, &old_name).unwrap().unwrap();
    let mut plan = plan::build(
        &layout,
        &manifest,
        &git::System,
        &source,
        old_name.clone(),
        Some(BranchName::new("new-branch").unwrap()),
    )
    .unwrap()
    .0;

    use crate::domain::name::RepoName;
    plan.repos.push(super::plan::RepoRenamePlan {
        repo: RepoName::new("acme").unwrap(),
        old_worktree: layout.repo_worktree(&RepoName::new("acme").unwrap(), &source.branch),
        new_worktree: layout.repo_worktree(
            &RepoName::new("acme").unwrap(),
            &BranchName::new("new-branch").unwrap(),
        ),
        old_branch: source.branch.clone(),
        new_branch: BranchName::new("new-branch").unwrap(),
        old_remote: Some("origin".to_owned()),
        old_remote_tip: Some("old-tip-sha".to_owned()),
    });

    let dummy_git = DummyRemoteGit::default();
    let res = steps::undo_step(
        &layout,
        &manifest,
        &dummy_git,
        &plan,
        steps::Step::RemoteOps,
    );
    assert_eq!(res.unwrap(), steps::Step::MoveWorktrees);

    let pushed = dummy_git.pushed.lock().unwrap();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].1, "origin");
    assert_eq!(pushed[0].2, "old-tip-sha");
    assert_eq!(pushed[0].3, format!("refs/heads/{}", source.branch));

    let deleted = dummy_git.deleted.lock().unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].1, "origin");
    assert_eq!(deleted[0].2, "new-branch");
    assert_eq!(deleted[0].3, "new-tip-sha");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FailOp {
    MoveWorktree,
    RenameBranch,
    Push,
    DeleteRemoteBranch,
    RemoteBranchTip,
}

struct FailingGit {
    dummy: DummyRemoteGit,
    fail_op: std::sync::Mutex<Option<(FailOp, usize)>>,
    op_counts: std::sync::Mutex<std::collections::HashMap<FailOp, usize>>,
}

impl FailingGit {
    fn new(fail_op: FailOp, index: usize) -> Self {
        Self {
            dummy: DummyRemoteGit::default(),
            fail_op: std::sync::Mutex::new(Some((fail_op, index))),
            op_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn should_fail(&self, op: FailOp) -> bool {
        let mut counts = self.op_counts.lock().unwrap();
        let count = counts.entry(op).or_insert(0);
        let current = *count;
        *count += 1;

        if matches!(*self.fail_op.lock().unwrap(), Some((target_op, target_idx)) if target_op == op && current == target_idx)
        {
            return true;
        }
        false
    }
}

impl git::Git for FailingGit {
    fn protect_default_branch(
        &self,
        git_dir: &Utf8Path,
        default_worktree: &Utf8Path,
        default_branch: &str,
    ) -> Result<git::protect::Protection, git::Error> {
        self.dummy
            .protect_default_branch(git_dir, default_worktree, default_branch)
    }
    fn target_state(&self, path: &Utf8Path) -> Result<git::TargetState, git::Error> {
        self.dummy.target_state(path)
    }
    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.head_branch(git_dir)
    }
    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, git::Error> {
        self.dummy.worktree_git_dir(path)
    }
    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.clone_bare(url, dest)
    }
    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.ensure_remote_tracking(git_dir)
    }
    fn add_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        branch: &str,
    ) -> Result<(), git::Error> {
        self.dummy.add_worktree(git_dir, dest, branch)
    }
    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.fetch(git_dir)
    }
    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, git::Error> {
        self.dummy.list_branches(git_dir)
    }
    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), git::Error> {
        self.dummy
            .create_branch_and_worktree(git_dir, branch, from_branch, dest)
    }
    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.fetch_branch(worktree, branch)
    }
    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.fast_forward(worktree)
    }
    fn remove_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.remove_worktree(git_dir, dest)
    }
    fn add_detached_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.dummy.add_detached_worktree(git_dir, dest, revision)
    }
    fn create_branch(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.dummy.create_branch(git_dir, branch, revision)
    }
    fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.delete_branch(git_dir, branch)
    }
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), git::Error> {
        self.dummy.merge_no_ff(worktree, source)
    }
    fn squash_merge(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), git::Error> {
        self.dummy.squash_merge(worktree, source, message)
    }
    fn fast_forward_to(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.dummy.fast_forward_to(worktree, revision)
    }
    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, git::Error> {
        self.dummy.worktree_dirty(path)
    }
    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.diff_worktree(path)
    }
    fn commit_patch_id(&self, worktree: &Utf8Path, commit: &str) -> Result<String, git::Error> {
        self.dummy.commit_patch_id(worktree, commit)
    }
    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.dummy.changed_paths(path)
    }
    fn head_commit(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.head_commit(path)
    }
    fn paths_committed_since(
        &self,
        path: &Utf8Path,
        since: &str,
    ) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.dummy.paths_committed_since(path, since)
    }
    fn path_at_commit(
        &self,
        worktree: &Utf8Path,
        commit: &str,
        path: &Utf8Path,
    ) -> Result<Option<git::BlobEvidence>, git::Error> {
        self.dummy.path_at_commit(worktree, commit, path)
    }
    fn commits_ahead(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<u64, git::Error> {
        self.dummy.commits_ahead(git_dir, base, branch)
    }
    fn is_ancestor(
        &self,
        git_dir: &Utf8Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, git::Error> {
        self.dummy.is_ancestor(git_dir, ancestor, descendant)
    }
    fn merge_base(&self, git_dir: &Utf8Path, a: &str, b: &str) -> Result<String, git::Error> {
        self.dummy.merge_base(git_dir, a, b)
    }
    fn divergence(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<git::Divergence, git::Error> {
        self.dummy.divergence(git_dir, base, branch)
    }
    fn diff_patch_id(
        &self,
        worktree: &Utf8Path,
        base: &str,
        tip: &str,
    ) -> Result<String, git::Error> {
        self.dummy.diff_patch_id(worktree, base, tip)
    }
    fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.dummy.reset_hard(worktree, revision)
    }
    fn revision_commit(&self, git_dir: &Utf8Path, revision: &str) -> Result<String, git::Error> {
        self.dummy.revision_commit(git_dir, revision)
    }
    fn remote_branch_tip(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, git::Error> {
        if self.should_fail(FailOp::RemoteBranchTip) {
            return Err(git::Error::Refused {
                command: "remote_branch_tip".to_owned(),
                detail: "injected failure".to_owned(),
            });
        }
        self.dummy.remote_branch_tip(git_dir, remote, branch)
    }
    fn push(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        from: &str,
        to: &str,
    ) -> Result<(), git::Error> {
        if self.should_fail(FailOp::Push) {
            return Err(git::Error::Refused {
                command: "push".to_owned(),
                detail: "injected failure".to_owned(),
            });
        }
        self.dummy.push(git_dir, remote, from, to)
    }
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.rebase_branch(worktree, branch)
    }
    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.abort_rebase(worktree)
    }
    fn rename_branch(&self, git_dir: &Utf8Path, from: &str, to: &str) -> Result<(), git::Error> {
        if self.should_fail(FailOp::RenameBranch) {
            return Err(git::Error::Refused {
                command: "rename_branch".to_owned(),
                detail: "injected failure".to_owned(),
            });
        }
        self.dummy.rename_branch(git_dir, from, to)
    }
    fn move_worktree(
        &self,
        git_dir: &Utf8Path,
        from: &Utf8Path,
        to: &Utf8Path,
    ) -> Result<(), git::Error> {
        if self.should_fail(FailOp::MoveWorktree) {
            return Err(git::Error::Refused {
                command: "move_worktree".to_owned(),
                detail: "injected failure".to_owned(),
            });
        }
        self.dummy.move_worktree(git_dir, from, to)
    }
    fn publish_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        at: &str,
    ) -> Result<(), git::Error> {
        self.dummy
            .publish_remote_branch(git_dir, remote, branch, at)
    }
    fn delete_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        expected_tip: &str,
    ) -> Result<(), git::Error> {
        if self.should_fail(FailOp::DeleteRemoteBranch) {
            return Err(git::Error::Refused {
                command: "delete_remote_branch".to_owned(),
                detail: "injected failure".to_owned(),
            });
        }
        self.dummy
            .delete_remote_branch(git_dir, remote, branch, expected_tip)
    }
}

struct RaceGit {
    system: git::System,
    divergent_branch: String,
    divergent_tip: String,
}

impl RaceGit {
    fn new(divergent_branch: &str, divergent_tip: &str) -> Self {
        Self {
            system: git::System,
            divergent_branch: divergent_branch.to_owned(),
            divergent_tip: divergent_tip.to_owned(),
        }
    }
}

impl git::Git for RaceGit {
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
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), git::Error> {
        self.system.merge_no_ff(worktree, source)
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
        if branch == self.divergent_branch {
            Ok(Some(self.divergent_tip.clone()))
        } else if branch.contains("old") {
            Ok(Some("sha-12345".to_owned()))
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
        self.system.push(git_dir, remote, from, to)
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
        self.system
            .delete_remote_branch(git_dir, remote, branch, expected_tip)
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
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

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
        },
    )
    .unwrap();

    let old_name = FeatureName::new(old_name_str).unwrap();
    let new_name = FeatureName::new(new_name_str).unwrap();
    let old_feature = Feature::read(&layout, &old_name).unwrap().unwrap();
    let old_branch = old_feature.branch.clone();
    let new_branch = BranchName::new(new_branch_str).unwrap();

    let old_plan_dir = layout.plan_dir(&old_name);
    fs::ensure_dir(&old_plan_dir).unwrap();

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
        Some(tc.new_branch.clone()),
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

#[test]
fn test_resume_forward_from_every_checkpoint() {
    let checkpoints = [
        steps::Step::Initialize,
        steps::Step::RenameBranches,
        steps::Step::MoveWorktrees,
        steps::Step::RemoteOps,
        steps::Step::MoveFeatureDir,
        steps::Step::UpdateChildren,
        steps::Step::MoveSessions,
        steps::Step::MovePlans,
    ];

    for (idx, &step_checkpoint) in checkpoints.iter().enumerate() {
        let old_name_str = format!("fw-old-{idx}");
        let new_name_str = format!("fw-new-{idx}");
        let new_branch_str = format!("feat/fw-new-b-{idx}");

        let tc = setup_test_hall(&old_name_str, &new_name_str, &new_branch_str);
        let git = git::System;
        let plan = build_test_plan(&tc, &git);

        for &s in &checkpoints[..idx] {
            steps::perform_step(&tc.layout, &tc.manifest, &git, &plan, s).unwrap();
        }

        let m_path = steps::marker_path(&plan);
        if let Some(parent) = m_path.parent() {
            fs::ensure_dir(parent).unwrap();
        }
        let transition = steps::Transition {
            version: 1,
            plan: plan.clone(),
            direction: steps::Direction::Forward,
            step: step_checkpoint,
        };
        json::write_canonical(&m_path, &transition).unwrap();

        let report = steps::resume(&tc.layout, &tc.manifest, &git, &m_path, transition).unwrap();
        assert!(report.is_clean());

        assert!(Feature::read(&tc.layout, &tc.old_name).unwrap().is_none());
        let updated = Feature::read(&tc.layout, &tc.new_name).unwrap().unwrap();
        assert_eq!(updated.name, tc.new_name);
        assert_eq!(updated.branch, tc.new_branch);

        for repo_plan in &plan.repos {
            assert!(fs::is_dir(&repo_plan.new_worktree).unwrap());
        }

        let child_name = FeatureName::new("child-feat").unwrap();
        let child = Feature::read(&tc.layout, &child_name).unwrap().unwrap();
        assert_eq!(child.parent, Some(tc.new_name.clone()));

        let sessions = lookup::list_feature(&tc.layout, &tc.new_name).unwrap();
        assert_eq!(sessions.len(), 1);
        let s_ref = &sessions[0];
        assert_eq!(
            s_ref.state.as_ref().unwrap().feature,
            Some(tc.new_name.clone())
        );
        assert!(fs::is_dir(&s_ref.view_dir).unwrap());

        assert!(fs::is_dir(&plan.new_plan_dir).unwrap());
        assert!(!fs::is_dir(&plan.old_plan_dir).unwrap());

        assert!(!fs::is_file(&m_path).unwrap());
        assert!(!fs::is_file(&plan.old_dir.join(".renaming")).unwrap());
        assert!(!fs::is_file(&plan.new_dir.join(".renaming")).unwrap());
    }
}

#[test]
fn test_resume_rollback_from_every_checkpoint() {
    let checkpoints = [
        steps::Step::Initialize,
        steps::Step::RenameBranches,
        steps::Step::MoveWorktrees,
        steps::Step::RemoteOps,
        steps::Step::MoveFeatureDir,
        steps::Step::UpdateChildren,
        steps::Step::MoveSessions,
        steps::Step::MovePlans,
    ];

    for (idx, &step_checkpoint) in checkpoints.iter().enumerate() {
        let old_name_str = format!("rb-old-{idx}");
        let new_name_str = format!("rb-new-{idx}");
        let new_branch_str = format!("feat/rb-new-b-{idx}");

        let tc = setup_test_hall(&old_name_str, &new_name_str, &new_branch_str);
        let git = git::System;
        let plan = build_test_plan(&tc, &git);

        for &s in &checkpoints[..=idx] {
            steps::perform_step(&tc.layout, &tc.manifest, &git, &plan, s).unwrap();
        }

        let m_path = steps::marker_path(&plan);
        if let Some(parent) = m_path.parent() {
            fs::ensure_dir(parent).unwrap();
        }
        let transition = steps::Transition {
            version: 1,
            plan: plan.clone(),
            direction: steps::Direction::RollingBack,
            step: step_checkpoint,
        };
        json::write_canonical(&m_path, &transition).unwrap();

        let res = steps::resume(&tc.layout, &tc.manifest, &git, &m_path, transition);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.status, Status::Blocked);
        assert_eq!(err.code, "rename.rolled_back");

        assert!(Feature::read(&tc.layout, &tc.new_name).unwrap().is_none());
        let restored = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
        assert_eq!(restored.name, tc.old_name);
        assert_eq!(restored.branch, tc.old_branch);

        for repo_plan in &plan.repos {
            assert!(fs::is_dir(&repo_plan.old_worktree).unwrap());
        }

        let child_name = FeatureName::new("child-feat").unwrap();
        let child = Feature::read(&tc.layout, &child_name).unwrap().unwrap();
        assert_eq!(child.parent, Some(tc.old_name.clone()));

        let sessions = lookup::list_feature(&tc.layout, &tc.old_name).unwrap();
        assert_eq!(sessions.len(), 1);
        let s_ref = &sessions[0];
        assert_eq!(
            s_ref.state.as_ref().unwrap().feature,
            Some(tc.old_name.clone())
        );
        assert!(fs::is_dir(&s_ref.view_dir).unwrap());

        assert!(fs::is_dir(&plan.old_plan_dir).unwrap());
        assert!(!fs::is_dir(&plan.new_plan_dir).unwrap());

        assert!(!fs::is_file(&m_path).unwrap());
        assert!(!fs::is_file(&plan.old_dir.join(".renaming")).unwrap());
        assert!(!fs::is_file(&plan.new_dir.join(".renaming")).unwrap());
    }
}

#[test]
fn test_forward_failure_triggers_rollback() {
    let git_healthy = git::System;

    // 1. Failure at MoveWorktrees
    {
        let tc = setup_test_hall("fail-mw-old", "fail-mw-new", "feat/fail-mw-b");
        let plan = build_test_plan(&tc, &git_healthy);

        let failing_git = FailingGit::new(FailOp::MoveWorktree, 0);
        let res = steps::run(&tc.layout, &tc.manifest, &failing_git, plan.clone());
        assert!(res.is_err());

        let (m_path, transition) = steps::find_transition(&tc.layout, &tc.old_name)
            .unwrap()
            .expect("transition marker should exist after failure");
        assert_eq!(transition.direction, steps::Direction::RollingBack);
        assert_eq!(transition.step, steps::Step::MoveWorktrees);

        let res_resume = steps::resume(&tc.layout, &tc.manifest, &git_healthy, &m_path, transition);
        assert!(res_resume.is_err());
        let err = res_resume.unwrap_err();
        assert_eq!(err.code, "rename.rolled_back");

        assert!(Feature::read(&tc.layout, &tc.new_name).unwrap().is_none());
        let restored = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
        assert_eq!(restored.name, tc.old_name);
        assert_eq!(restored.branch, tc.old_branch);
        assert!(fs::is_dir(&plan.old_plan_dir).unwrap());
        assert!(!fs::is_file(&m_path).unwrap());
    }

    // 2. Failure at RemoteOps
    {
        let tc = setup_test_hall("fail-ro-old", "fail-ro-new", "feat/fail-ro-b");
        let mut plan = build_test_plan(&tc, &git_healthy);
        plan.repos[0].old_remote = Some("origin".to_owned());
        plan.repos[0].old_remote_tip = Some("sha-1".to_owned());

        let failing_git = FailingGit::new(FailOp::RemoteBranchTip, 0);
        let res = steps::run(&tc.layout, &tc.manifest, &failing_git, plan.clone());
        assert!(res.is_err());

        let (m_path, transition) = steps::find_transition(&tc.layout, &tc.old_name)
            .unwrap()
            .expect("transition marker should exist after failure");
        assert_eq!(transition.direction, steps::Direction::RollingBack);
        assert_eq!(transition.step, steps::Step::RemoteOps);

        let dummy_git = DummyRemoteGit::default();
        let res_resume = steps::resume(&tc.layout, &tc.manifest, &dummy_git, &m_path, transition);
        assert!(res_resume.is_err());
        let err = res_resume.unwrap_err();
        assert_eq!(err.code, "rename.rolled_back");

        assert!(Feature::read(&tc.layout, &tc.new_name).unwrap().is_none());
        let restored = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
        assert_eq!(restored.name, tc.old_name);
        assert_eq!(restored.branch, tc.old_branch);
        assert!(fs::is_dir(&plan.old_plan_dir).unwrap());
        assert!(!fs::is_file(&m_path).unwrap());
    }
}

#[test]
fn test_remote_race_aborts_without_mutation() {
    let tc = setup_test_hall("race-old", "race-new", "feat/race-new-b");
    let git_healthy = git::System;
    let mut plan = build_test_plan(&tc, &git_healthy);

    for repo_plan in &mut plan.repos {
        repo_plan.old_remote = Some("origin".to_owned());
        repo_plan.old_remote_tip = Some("sha-12345".to_owned());
    }

    let race_git = RaceGit::new("feat/race-new-b", "divergent-tip-sha");

    let res = steps::run(&tc.layout, &tc.manifest, &race_git, plan.clone());
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "rename.remote_race");

    let (m_path, transition) = steps::find_transition(&tc.layout, &tc.old_name)
        .unwrap()
        .expect("transition marker should exist after race error");
    assert_eq!(transition.direction, steps::Direction::RollingBack);

    let dummy_git = DummyRemoteGit::default();
    let res_resume = steps::resume(&tc.layout, &tc.manifest, &dummy_git, &m_path, transition);
    assert!(res_resume.is_err());
    let err_rb = res_resume.unwrap_err();
    assert_eq!(err_rb.code, "rename.rolled_back");

    assert!(Feature::read(&tc.layout, &tc.new_name).unwrap().is_none());
    let restored = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
    assert_eq!(restored.name, tc.old_name);
    assert_eq!(restored.branch, tc.old_branch);

    for repo_plan in &plan.repos {
        assert!(fs::is_dir(&repo_plan.old_worktree).unwrap());
    }

    let child_name = FeatureName::new("child-feat").unwrap();
    let child = Feature::read(&tc.layout, &child_name).unwrap().unwrap();
    assert_eq!(child.parent, Some(tc.old_name.clone()));

    assert!(fs::is_dir(&plan.old_plan_dir).unwrap());
    assert!(!fs::is_file(&m_path).unwrap());
}

struct PublishedRemoteGit {
    dummy: DummyRemoteGit,
}

impl git::Git for PublishedRemoteGit {
    fn protect_default_branch(
        &self,
        git_dir: &Utf8Path,
        default_worktree: &Utf8Path,
        default_branch: &str,
    ) -> Result<git::protect::Protection, git::Error> {
        self.dummy
            .protect_default_branch(git_dir, default_worktree, default_branch)
    }
    fn target_state(&self, path: &Utf8Path) -> Result<git::TargetState, git::Error> {
        self.dummy.target_state(path)
    }
    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.head_branch(git_dir)
    }
    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, git::Error> {
        self.dummy.worktree_git_dir(path)
    }
    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.clone_bare(url, dest)
    }
    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.ensure_remote_tracking(git_dir)
    }
    fn add_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        branch: &str,
    ) -> Result<(), git::Error> {
        self.dummy.add_worktree(git_dir, dest, branch)
    }
    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.fetch(git_dir)
    }
    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, git::Error> {
        self.dummy.list_branches(git_dir)
    }
    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), git::Error> {
        self.dummy
            .create_branch_and_worktree(git_dir, branch, from_branch, dest)
    }
    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.fetch_branch(worktree, branch)
    }
    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.fast_forward(worktree)
    }
    fn remove_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.remove_worktree(git_dir, dest)
    }
    fn add_detached_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.dummy.add_detached_worktree(git_dir, dest, revision)
    }
    fn create_branch(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        revision: &str,
    ) -> Result<(), git::Error> {
        self.dummy.create_branch(git_dir, branch, revision)
    }
    fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.delete_branch(git_dir, branch)
    }
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), git::Error> {
        self.dummy.merge_no_ff(worktree, source)
    }
    fn squash_merge(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), git::Error> {
        self.dummy.squash_merge(worktree, source, message)
    }
    fn fast_forward_to(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.dummy.fast_forward_to(worktree, revision)
    }
    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, git::Error> {
        self.dummy.worktree_dirty(path)
    }
    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.diff_worktree(path)
    }
    fn commit_patch_id(&self, worktree: &Utf8Path, commit: &str) -> Result<String, git::Error> {
        self.dummy.commit_patch_id(worktree, commit)
    }
    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.dummy.changed_paths(path)
    }
    fn head_commit(&self, path: &Utf8Path) -> Result<String, git::Error> {
        self.dummy.head_commit(path)
    }
    fn paths_committed_since(
        &self,
        path: &Utf8Path,
        since: &str,
    ) -> Result<Vec<Utf8PathBuf>, git::Error> {
        self.dummy.paths_committed_since(path, since)
    }
    fn path_at_commit(
        &self,
        worktree: &Utf8Path,
        commit: &str,
        path: &Utf8Path,
    ) -> Result<Option<git::BlobEvidence>, git::Error> {
        self.dummy.path_at_commit(worktree, commit, path)
    }
    fn commits_ahead(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<u64, git::Error> {
        self.dummy.commits_ahead(git_dir, base, branch)
    }
    fn is_ancestor(
        &self,
        git_dir: &Utf8Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, git::Error> {
        self.dummy.is_ancestor(git_dir, ancestor, descendant)
    }
    fn merge_base(&self, git_dir: &Utf8Path, a: &str, b: &str) -> Result<String, git::Error> {
        self.dummy.merge_base(git_dir, a, b)
    }
    fn divergence(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<git::Divergence, git::Error> {
        self.dummy.divergence(git_dir, base, branch)
    }
    fn diff_patch_id(
        &self,
        worktree: &Utf8Path,
        base: &str,
        tip: &str,
    ) -> Result<String, git::Error> {
        self.dummy.diff_patch_id(worktree, base, tip)
    }
    fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), git::Error> {
        self.dummy.reset_hard(worktree, revision)
    }
    fn revision_commit(&self, git_dir: &Utf8Path, revision: &str) -> Result<String, git::Error> {
        self.dummy.revision_commit(git_dir, revision)
    }
    fn remote_branch_tip(
        &self,
        _git_dir: &Utf8Path,
        _remote: &str,
        _branch: &str,
    ) -> Result<Option<String>, git::Error> {
        Ok(Some("published-tip-sha".to_owned()))
    }
    fn push(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        from: &str,
        to: &str,
    ) -> Result<(), git::Error> {
        self.dummy.push(git_dir, remote, from, to)
    }
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.rebase_branch(worktree, branch)
    }
    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), git::Error> {
        self.dummy.abort_rebase(worktree)
    }
    fn rename_branch(&self, git_dir: &Utf8Path, from: &str, to: &str) -> Result<(), git::Error> {
        self.dummy.rename_branch(git_dir, from, to)
    }
    fn move_worktree(
        &self,
        git_dir: &Utf8Path,
        from: &Utf8Path,
        to: &Utf8Path,
    ) -> Result<(), git::Error> {
        self.dummy.move_worktree(git_dir, from, to)
    }
    fn publish_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        at: &str,
    ) -> Result<(), git::Error> {
        self.dummy
            .publish_remote_branch(git_dir, remote, branch, at)
    }
    fn delete_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        expected_tip: &str,
    ) -> Result<(), git::Error> {
        self.dummy
            .delete_remote_branch(git_dir, remote, branch, expected_tip)
    }
}

#[test]
fn preflight_pr_check_only_runs_when_branch_is_published() {
    let tc = setup_test_hall("pr-check-old", "pr-check-new", "feat/pr-check-new-b");
    let repo_name = crate::domain::name::RepoName::new("api").unwrap();
    let old_worktree = tc.layout.repo_worktree(&repo_name, &tc.old_branch);
    let bare = tc.layout.repo_bare(&repo_name);
    let sys_git = git::System;
    if sys_git
        .revision_commit(&bare, tc.old_branch.as_str())
        .is_err()
    {
        let _ = sys_git.create_branch(&bare, tc.old_branch.as_str(), "main");
    }
    if !fs::is_dir(&old_worktree).unwrap_or(false) {
        let _ = sys_git.add_worktree(&bare, &old_worktree, tc.old_branch.as_str());
    }

    let mut source = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
    source.promotions.insert(
        repo_name.clone(),
        crate::domain::feature::Promotion {
            worktree: crate::domain::feature::WorktreeState::Ready,
            base: Some(BranchName::new("main").unwrap()),
            integration_receipt: None,
        },
    );

    // Remove the bare directory so that `find_pull_request` (which runs `gh` in cwd=bare)
    // fails strictly if invoked.
    std::fs::remove_dir_all(&bare).unwrap();

    // 1. Unpublished branch (remote_branch_tip -> None): PR check skipped, preflight passes without PR blocker
    let dummy_git = DummyRemoteGit::default();
    let (_plan, blockers_unpub) = plan::build(
        &tc.layout,
        &tc.manifest,
        &dummy_git,
        &source,
        tc.new_name.clone(),
        Some(tc.new_branch.clone()),
    )
    .unwrap();
    assert!(
        blockers_unpub.iter().all(|b| b.scope != "pull-request"),
        "Unpublished branch should not trigger PR check blockers: {blockers_unpub:?}"
    );

    // 2. Published branch (remote_branch_tip -> Some): PR check runs and fails, causing strict PR blocker
    let published_git = PublishedRemoteGit {
        dummy: DummyRemoteGit::default(),
    };
    let (_plan, blockers_pub) = plan::build(
        &tc.layout,
        &tc.manifest,
        &published_git,
        &source,
        tc.new_name.clone(),
        Some(tc.new_branch.clone()),
    )
    .unwrap();

    let pr_blocker = blockers_pub.iter().find(|b| b.scope == "pull-request");
    assert!(
        pr_blocker.is_some(),
        "Published branch with failing gh must record a pull-request blocker: {blockers_pub:?}"
    );
    assert!(
        pr_blocker
            .unwrap()
            .explanation
            .contains("Failed to check open PRs"),
        "Blocker explanation should indicate gh failure: {:?}",
        pr_blocker.unwrap().explanation
    );
}
