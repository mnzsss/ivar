use super::*;

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
            pr_url: None,
            delivered_sha: None,
            integration_receipt: None,
            reason: None,
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
        Some(&tc.new_branch.clone()),
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
        Some(&tc.new_branch.clone()),
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
