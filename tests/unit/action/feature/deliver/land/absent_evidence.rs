use super::super::fixture::*;
use super::super::*;

#[test]
fn absence_of_preview_evidence_none_err_skips_whole_batch() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let mut preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview")
    .value
    .preview;

    // Simulate preview having no remote_default_tip evidence
    preview.repos[0].remote_default_tip = None;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &preview,
    )
    .unwrap();

    // Mock git that returns Err for remote_branch_tip
    struct ErrRemoteGit(crate::git::System);
    impl crate::git::Git for ErrRemoteGit {
        fn protect_default_branch(
            &self,
            git_dir: &Utf8Path,
            default_worktree: &Utf8Path,
            default_branch: &str,
        ) -> Result<crate::git::protect::Protection, crate::git::Error> {
            self.0
                .protect_default_branch(git_dir, default_worktree, default_branch)
        }
        fn target_state(
            &self,
            path: &Utf8Path,
        ) -> Result<crate::git::TargetState, crate::git::Error> {
            self.0.target_state(path)
        }
        fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_branch(git_dir)
        }
        fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, crate::git::Error> {
            self.0.worktree_git_dir(path)
        }
        fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.clone_bare(url, dest)
        }
        fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.ensure_remote_tracking(git_dir)
        }
        fn add_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_worktree(git_dir, dest, branch)
        }
        fn fetch(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fetch(git_dir)
        }
        fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, crate::git::Error> {
            self.0.list_branches(git_dir)
        }
        fn create_branch_and_worktree(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            from_branch: &str,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0
                .create_branch_and_worktree(git_dir, branch, from_branch, dest)
        }
        fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.fetch_branch(worktree, branch)
        }
        fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fast_forward(worktree)
        }
        fn remove_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.remove_worktree(git_dir, dest)
        }
        fn add_detached_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_detached_worktree(git_dir, dest, revision)
        }
        fn create_branch(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.create_branch(git_dir, branch, revision)
        }
        fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.delete_branch(git_dir, branch)
        }
        fn merge_no_ff(
            &self,
            worktree: &Utf8Path,
            source: &str,
            message: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.merge_no_ff(worktree, source, message)
        }
        fn squash_merge(
            &self,
            worktree: &Utf8Path,
            source: &str,
            message: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.squash_merge(worktree, source, message)
        }
        fn fast_forward_to(
            &self,
            worktree: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.fast_forward_to(worktree, revision)
        }
        fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, crate::git::Error> {
            self.0.worktree_dirty(path)
        }
        fn diff_worktree(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.diff_worktree(path)
        }
        fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.changed_paths(path)
        }
        fn head_commit(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_commit(path)
        }
        fn paths_committed_since(
            &self,
            path: &Utf8Path,
            since: &str,
        ) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.paths_committed_since(path, since)
        }
        fn path_at_commit(
            &self,
            git_dir: &Utf8Path,
            commit: &str,
            path: &Utf8Path,
        ) -> Result<Option<crate::git::BlobEvidence>, crate::git::Error> {
            self.0.path_at_commit(git_dir, commit, path)
        }
        fn commits_ahead(
            &self,
            git_dir: &Utf8Path,
            base: &str,
            branch: &str,
        ) -> Result<u64, crate::git::Error> {
            self.0.commits_ahead(git_dir, base, branch)
        }
        fn is_ancestor(
            &self,
            git_dir: &Utf8Path,
            ancestor: &str,
            descendant: &str,
        ) -> Result<bool, crate::git::Error> {
            self.0.is_ancestor(git_dir, ancestor, descendant)
        }
        fn divergence(
            &self,
            git_dir: &Utf8Path,
            local: &str,
            remote: &str,
        ) -> Result<crate::git::Divergence, crate::git::Error> {
            self.0.divergence(git_dir, local, remote)
        }
        fn merge_base(
            &self,
            git_dir: &Utf8Path,
            a: &str,
            b: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.merge_base(git_dir, a, b)
        }
        fn revision_commit(
            &self,
            git_dir: &Utf8Path,
            revision: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.revision_commit(git_dir, revision)
        }
        fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), crate::git::Error> {
            self.0.reset_hard(worktree, revision)
        }
        fn remote_branch_tip(
            &self,
            _git_dir: &Utf8Path,
            _remote: &str,
            _branch: &str,
        ) -> Result<Option<String>, crate::git::Error> {
            Err(crate::git::Error::Refused {
                command: "git ls-remote".to_owned(),
                detail: "simulated error".to_owned(),
            })
        }
        fn push(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.push(git_dir, remote, from, to)
        }
        fn commit_patch_id(
            &self,
            worktree: &Utf8Path,
            commit: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.commit_patch_id(worktree, commit)
        }
        fn diff_patch_id(
            &self,
            worktree: &Utf8Path,
            base: &str,
            tip: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.diff_patch_id(worktree, base, tip)
        }
        fn rebase_branch(
            &self,
            worktree: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rebase_branch(worktree, branch)
        }
        fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.abort_rebase(worktree)
        }
        fn rename_branch(
            &self,
            git_dir: &Utf8Path,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rename_branch(git_dir, from, to)
        }
        fn move_worktree(
            &self,
            git_dir: &Utf8Path,
            from: &Utf8Path,
            to: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.move_worktree(git_dir, from, to)
        }
        fn publish_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            at: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.publish_remote_branch(git_dir, remote, branch, at)
        }
        fn delete_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            expected_tip: &str,
        ) -> Result<(), crate::git::Error> {
            self.0
                .delete_remote_branch(git_dir, remote, branch, expected_tip)
        }
    }

    let mock_git = ErrRemoteGit(crate::git::System);
    let mut warnings = Vec::new();
    let results =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "None/Err must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved")
    );
}

#[test]
fn land_absent_preview_evidence_reports_absent_at_preview_not_moved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["update-ref", "-d", "refs/heads/main"]);

    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    assert!(preview.repos[0].remote_default_tip.is_none());

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("deliver with absent preview evidence returns results with unmerged status");

    assert_eq!(out.value.land.len(), 1);
    assert!(!out.value.land[0].merged);
    assert_eq!(
        out.value.land[0].detail.as_deref(),
        Some("remote default branch absent at preview")
    );

    let warning = out
        .warnings
        .iter()
        .find(|w| w.code == "deliver.land_remote_moved")
        .expect("land_remote_moved warning emitted");

    assert!(
        warning.what.contains("absent at preview"),
        "warning message must state 'absent at preview', got: {}",
        warning.what
    );
    assert!(
        !warning.what.contains("moved (preview expected"),
        "warning message must not report 'moved' when evidence was absent"
    );
}

#[test]
fn land_remote_disappeared_reports_disappeared_from_remote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &land_preview,
    )
    .unwrap();

    git_stdout(&origins, &["update-ref", "-d", "refs/heads/main"]);

    let mut warnings = Vec::new();
    let results = crate::action::feature::deliver::land::execute(
        &crate::git::System,
        &layout,
        &plans,
        &mut warnings,
    )
    .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].merged);
    assert_eq!(
        results[0].detail.as_deref(),
        Some("remote default branch disappeared from remote")
    );

    let warning = warnings
        .iter()
        .find(|w| w.code == "deliver.land_remote_moved")
        .expect("warning emitted");

    assert!(warning.what.contains("disappeared from remote"));
}
