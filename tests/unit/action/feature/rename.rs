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
use crate::action::feature::create::create as create_action;
use crate::action::feature::create::CreateInput;
use crate::action::session::lookup;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::Status;
use crate::infra::{fs, json};
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
    fn add_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path, branch: &str) -> Result<(), git::Error> {
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
        self.system.create_branch_and_worktree(git_dir, branch, from_branch, dest)
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
    fn create_branch(&self, git_dir: &Utf8Path, branch: &str, revision: &str) -> Result<(), git::Error> {
        self.system.create_branch(git_dir, branch, revision)
    }
    fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.system.delete_branch(git_dir, branch)
    }
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), git::Error> {
        self.system.merge_no_ff(worktree, source)
    }
    fn squash_merge(&self, worktree: &Utf8Path, source: &str, message: &str) -> Result<(), git::Error> {
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
    fn paths_committed_since(&self, path: &Utf8Path, since: &str) -> Result<Vec<Utf8PathBuf>, git::Error> {
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
    fn commits_ahead(&self, git_dir: &Utf8Path, base: &str, branch: &str) -> Result<u64, git::Error> {
        self.system.commits_ahead(git_dir, base, branch)
    }
    fn is_ancestor(&self, git_dir: &Utf8Path, ancestor: &str, descendant: &str) -> Result<bool, git::Error> {
        self.system.is_ancestor(git_dir, ancestor, descendant)
    }
    fn merge_base(&self, git_dir: &Utf8Path, a: &str, b: &str) -> Result<String, git::Error> {
        self.system.merge_base(git_dir, a, b)
    }
    fn divergence(&self, git_dir: &Utf8Path, base: &str, branch: &str) -> Result<git::Divergence, git::Error> {
        self.system.divergence(git_dir, base, branch)
    }
    fn diff_patch_id(&self, worktree: &Utf8Path, base: &str, tip: &str) -> Result<String, git::Error> {
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
    fn push(&self, git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), git::Error> {
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
    fn move_worktree(&self, git_dir: &Utf8Path, from: &Utf8Path, to: &Utf8Path) -> Result<(), git::Error> {
        self.system.move_worktree(git_dir, from, to)
    }
    fn publish_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        at: &str,
    ) -> Result<(), git::Error> {
        self.system.publish_remote_branch(git_dir, remote, branch, at)
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
        rename_input("record-old", Some("record-new"), Some("feat/record-new-branch")),
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
    println!("CHILD PARENT AFTER: {:?}", child_after.parent);
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
        new_worktree: layout.repo_worktree(&RepoName::new("acme").unwrap(), &BranchName::new("new-branch").unwrap()),
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
