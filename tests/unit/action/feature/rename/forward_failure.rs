use super::*;

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

        matches!(*self.fail_op.lock().unwrap(), Some((target_op, target_idx)) if target_op == op && current == target_idx)
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
    fn switch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), git::Error> {
        self.dummy.switch_branch(worktree, branch)
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
    fn merge_no_ff(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), git::Error> {
        self.dummy.merge_no_ff(worktree, source, message)
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

#[test]
fn test_forward_failure_triggers_rollback() {
    let git_healthy = git::System;

    // 1. Failure at MoveWorktrees
    {
        let tc = setup_test_hall("fail-mw-old", "fail-mw-new", "feat/fail-mw-b");
        let plan = build_test_plan(&tc, &git_healthy);

        let failing_git = FailingGit::new(FailOp::MoveWorktree, 0);
        let res = steps::run(&tc.layout, &tc.manifest, &failing_git, &plan.clone());
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
        assert!(fs::is_dir(&plan.old_dir).unwrap());
        assert!(!fs::is_file(&m_path).unwrap());
    }

    // 2. Failure at RemoteOps
    {
        let tc = setup_test_hall("fail-ro-old", "fail-ro-new", "feat/fail-ro-b");
        let mut plan = build_test_plan(&tc, &git_healthy);
        plan.repos[0].old_remote = Some("origin".to_owned());
        plan.repos[0].old_remote_tip = Some("sha-1".to_owned());

        let failing_git = FailingGit::new(FailOp::RemoteBranchTip, 0);
        let res = steps::run(&tc.layout, &tc.manifest, &failing_git, &plan.clone());
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
        assert!(fs::is_dir(&plan.old_dir).unwrap());
        assert!(!fs::is_file(&m_path).unwrap());
    }
}
