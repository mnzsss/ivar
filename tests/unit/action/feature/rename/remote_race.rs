use super::*;

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

    let res = steps::run(&tc.layout, &tc.manifest, &race_git, &plan.clone());
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

    assert!(fs::is_dir(&plan.old_dir).unwrap());
    assert!(!fs::is_file(&m_path).unwrap());
}
