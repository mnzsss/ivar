use super::super::fixture::*;
use super::super::*;

#[test]
fn a_matching_land_fingerprint_executes_the_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let applied = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint.clone()),
            ..Default::default()
        },
    );
    // Wave 3: land now executes; a push to a non-bare origin may warn but the
    // merge must have been attempted.
    match applied {
        Ok(out) => {
            assert!(
                out.value.land.iter().any(|r| r.merged),
                "at least one repo must merge"
            );
        }
        Err(failure) => {
            // fast-forward failure or mode mismatch — both surface clearly
            assert_ne!(
                failure.code, "deliver.land_not_implemented",
                "land_not_implemented must never fire after Wave 3",
            );
        }
    }
}

// -- land preview and blockers (Wave 2) ------------------------------------

#[test]
fn a_clean_land_merges_every_repo_and_pushes_each_default() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(
        feature_worktree.join("feature.txt"),
        "new feature content\n",
    )
    .unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "add feature.txt"]);
    let feature_tip = git_stdout(&feature_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("land apply");

    assert_eq!(out.value.land.len(), 1);
    assert!(out.value.land[0].merged);
    assert!(out.value.land[0].pushed);

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let default_tip = git_stdout(&default_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_eq!(
        default_tip, feature_tip,
        "default branch must equal feature tip"
    );

    assert!(
        feature_worktree.exists(),
        "feature worktree must still exist"
    );
}

#[test]
fn a_failed_push_is_a_warning_not_an_abort() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("feature.txt"), "content\n").unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "feature commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("a failed push must not abort the land");

    assert_eq!(out.value.land.len(), 1);
    assert!(
        out.value.land[0].merged,
        "merge stands even when push fails"
    );
    assert!(!out.value.land[0].pushed, "push failed");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_push_failed"),
        "push failure produces deliver.land_push_failed warning"
    );
}

#[test]
fn execute_failure_rolls_back_earlier_merges() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_api.join("api.txt"), "api\n").unwrap();
    git_stdout(&feature_api, &["add", "api.txt"]);
    git_stdout(&feature_api, &["commit", "-m", "api commit"]);

    let feature_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_web.join("web.txt"), "web\n").unwrap();
    git_stdout(&feature_web, &["add", "web.txt"]);
    git_stdout(&feature_web, &["commit", "-m", "web commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let api_default_before = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_git_dir = crate::git::System.worktree_git_dir(&web_default).unwrap();
    std::fs::write(web_git_dir.join("index.lock"), "lock").unwrap();

    let _failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("execute failure on web must return Err");

    let api_default_after = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(
        api_default_before, api_default_after,
        "repo api must be rolled back if repo web fails during execute"
    );
}

#[test]
fn remote_default_ahead_of_local_default_does_not_trigger_remote_moved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    // Push a commit to origin main so remote main is ahead of local main
    git_stdout(
        &origins,
        &["commit", "--allow-empty", "-m", "origin main ahead"],
    );

    // Feature branch on top of origin main
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    git_stdout(&feature_worktree, &["fetch", "origin"]);
    git_stdout(&feature_worktree, &["rebase", "origin/main"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("land apply");

    assert!(
        !out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "remote main did not move after preview; land_remote_moved must not fire"
    );
}

#[test]
fn failure_conventions_are_honored_for_land_system_failures() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("f.txt"), "f").unwrap();
    git_stdout(&feature_worktree, &["add", "f.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "f"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("preview");

    // Lock default worktree index to cause fast_forward_to failure
    let git_dir = crate::git::System
        .worktree_git_dir(&default_worktree)
        .unwrap();
    std::fs::write(git_dir.join("index.lock"), "lock").unwrap();

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("merge failure");

    assert_eq!(failure.code, "git.merge_ff_only_failed");
    assert!(failure.expected.is_some(), "expected must be populated");
    assert!(failure.actual.is_some(), "actual must be populated");
    assert!(
        !failure.fix_actions.is_empty(),
        "fix_actions must be populated"
    );
}

#[test]
fn absence_of_preview_evidence_none_none_skips_whole_batch() {
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

    // Mock git that returns Ok(None) for remote_branch_tip
    struct UnreachableRemoteGit(crate::git::System);
    impl crate::git::Git for UnreachableRemoteGit {
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
        fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
            self.0.merge_no_ff(worktree, source)
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
            Ok(None)
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

    let mock_git = UnreachableRemoteGit(crate::git::System);
    let mut warnings = Vec::new();
    let results =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "None/None must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved")
    );
}

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
        fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
            self.0.merge_no_ff(worktree, source)
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
fn expected_none_current_some_blocks_or_skips_whole_batch() {
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

    // Simulate preview having no remote_default_tip
    preview.repos[0].remote_default_tip = None;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &preview,
    )
    .unwrap();

    let mut warnings = Vec::new();
    let results = crate::action::feature::deliver::land::execute(
        &crate::git::System,
        &layout,
        &plans,
        &mut warnings,
    )
    .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "expected None when current is Some must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "must emit land_remote_moved warning"
    );
}

#[test]
fn remote_moved_on_second_repo_skips_or_blocks_whole_batch_writing_neither() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let origins_web = root.parent().unwrap().join("origins").join("web");
    git_stdout(
        &origins_web,
        &["config", "receive.denyCurrentBranch", "ignore"],
    );

    let before = snapshot_all_worktrees(&root);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    // Add commit to origin web after preview
    git_stdout(
        &origins_web,
        &["commit", "--allow-empty", "-m", "web origin moved"],
    );

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    );

    let after = snapshot_all_worktrees(&root);
    assert_eq!(
        before, after,
        "remote moved on repo 2 must write NEITHER repo 1 nor repo 2"
    );

    match out {
        Ok(report) => {
            assert!(
                report.value.land.iter().all(|r| !r.merged),
                "all repos must be unmerged"
            );
            assert!(
                report
                    .warnings
                    .iter()
                    .any(|w| w.code == "deliver.land_remote_moved"),
                "land_remote_moved warning must be emitted"
            );
        }
        Err(failure) => {
            assert!(
                failure.code == "deliver.fingerprint_mismatch"
                    || failure.code == "deliver.land_remote_moved",
                "must refuse when remote branch moves"
            );
        }
    }
}

#[test]
fn remote_moved_with_warning_skips_batch_and_emits_warning() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let origins_web = root.parent().unwrap().join("origins").join("web");
    git_stdout(
        &origins_web,
        &["config", "receive.denyCurrentBranch", "ignore"],
    );

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &land_preview.value.preview,
    )
    .unwrap();

    git_stdout(
        &origins_web,
        &["commit", "--allow-empty", "-m", "web origin moved"],
    );

    let mut warnings = Vec::new();
    let results = crate::action::feature::deliver::land::execute(
        &crate::git::System,
        &layout,
        &plans,
        &mut warnings,
    )
    .expect("execute");

    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "land_remote_moved warning must be present"
    );
    assert!(
        results.iter().all(|r| !r.merged),
        "entire batch must be skipped when remote moves"
    );
}

struct FailingRollbackGit(crate::git::System);

impl crate::git::Git for FailingRollbackGit {
    fn target_state(&self, path: &Utf8Path) -> Result<crate::git::TargetState, crate::git::Error> {
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
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
        self.0.merge_no_ff(worktree, source)
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
    fn reset_hard(&self, _worktree: &Utf8Path, _revision: &str) -> Result<(), crate::git::Error> {
        Err(crate::git::Error::Refused {
            command: "git reset --hard".to_owned(),
            detail: "simulated reset_hard failure for rollback test".to_owned(),
        })
    }
    fn remote_branch_tip(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, crate::git::Error> {
        self.0.remote_branch_tip(git_dir, remote, branch)
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
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
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

#[test]
fn rollback_failure_produces_land_rollback_failed_failure() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let feature_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_api.join("api.txt"), "api").unwrap();
    git_stdout(&feature_api, &["add", "api.txt"]);
    git_stdout(&feature_api, &["commit", "-m", "api commit"]);

    let feature_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_web.join("web.txt"), "web").unwrap();
    git_stdout(&feature_web, &["add", "web.txt"]);
    git_stdout(&feature_web, &["commit", "-m", "web commit"]);

    let preview = deliver(
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

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &preview,
    )
    .unwrap();

    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    // Lock web_default index so fast_forward_to fails during merge phase
    let web_git_dir = crate::git::System.worktree_git_dir(&web_default).unwrap();
    std::fs::write(web_git_dir.join("index.lock"), "lock").unwrap();

    let mock_git = FailingRollbackGit(crate::git::System);
    let mut warnings = Vec::new();
    let failure =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .expect_err("rollback failure must produce Failure");

    assert_eq!(failure.code, "deliver.land_rollback_failed");
    assert!(failure.expected.is_some());
    assert!(failure.actual.is_some());
    assert!(!failure.fix_actions.is_empty());
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

#[test]
fn land_merge_failure_fix_action_uses_feature_name() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let feature_name = "auth-v2";
    let custom_branch = "feat/auth-v2";
    create_action(
        &ctx,
        CreateInput {
            name: feature_name.to_owned(),
            branch: Some(custom_branch.to_owned()),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    promote::promote(
        &ctx,
        PromoteInput {
            feature: feature_name.to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: feature_name.to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        crate::action::plan::approve::approve(
            &ctx,
            crate::action::plan::approve::ApproveInput {
                feature: feature_name.to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new(feature_name).unwrap()).unwrap();
    let feature_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
    std::fs::write(feature_worktree.join("f.txt"), "f").unwrap();
    git_stdout(&feature_worktree, &["add", "f.txt"]);
    git_stdout(
        &feature_worktree,
        &["commit", "-m", "commit on feat/auth-v2"],
    );

    let land_preview = deliver(&ctx, land_preview_input(feature_name))
        .expect("land preview")
        .value
        .preview;

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let git_dir = crate::git::System
        .worktree_git_dir(&default_worktree)
        .unwrap();
    std::fs::write(git_dir.join("index.lock"), "lock").unwrap();

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: feature_name.to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("merge failure must return Err");

    assert_eq!(failure.code, "git.merge_ff_only_failed");
    let fix = failure
        .fix_actions
        .first()
        .expect("fix action must be present");
    assert!(
        fix.what.contains("ivar feature rebase auth-v2"),
        "fix action must name the feature, got: {}",
        fix.what
    );
}

#[test]
fn land_revalidates_head_movement_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_feature = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );

    // Make web_default at commit C2, and web_feature at C2 -> work
    git(&web_feature, &["commit", "--allow-empty", "-m", "c2"]);
    let c2 = git_stdout(&web_feature, &["rev-parse", "HEAD"]);
    git(&web_feature, &["commit", "--allow-empty", "-m", "work2"]);
    git(&web_default, &["reset", "--hard", c2.trim()]);

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api moves web_default HEAD between preflight and phase 2
    let manifest = read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone()
                    .with_checks(vec![format!("git -C {web_default} reset --hard HEAD~1")])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("HEAD movement during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_head_moved");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);
}

#[test]
fn land_revalidates_dirty_worktree_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api dirties web_default worktree between preflight and phase 2
    let manifest = read_manifest(&layout).unwrap();
    let dirty_path = web_default.join("dirty_test.txt");
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone()
                    .with_checks(vec![format!("sh -c 'echo dirty > {dirty_path}'")])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("dirty worktree during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_dirty_worktree");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);

    // Repo 2 dirty worktree is untouched
    assert!(dirty_path.exists());
    assert_eq!(std::fs::read_to_string(&dirty_path).unwrap(), "dirty\n");
}

#[test]
fn land_revalidates_fast_forward_and_rolls_back() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    let api_orig_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    // Verification check on api creates a non-fast-forward commit on web_default
    let manifest = read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone().with_checks(vec![format!(
                    "git -C {web_default} commit --allow-empty -m 'diverged'"
                )])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("non-fast-forward during phase 2 must refuse land and roll back");

    assert_eq!(failure.code, "deliver.land_not_fast_forward");

    // Repo 1 (api) was rolled back to api_orig_head
    let api_current_head = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(api_current_head, api_orig_head);
}
