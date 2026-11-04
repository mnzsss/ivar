use super::*;

// -- the empty hall --------------------------------------------------------

#[test]
fn syncing_a_hall_with_no_repos_sets_up_the_skeleton_and_the_managed_block() {
    let (_guard, root) = hall_with(&[]);
    // Simulate a hall whose instructions were never materialised (or were
    // deleted by hand): `sync` is what creates the canonical file and the
    // first alias.
    fs::remove_file(&root.join("HALL.md")).unwrap();
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(fs::is_dir(&root.join(".ivar/repos")).unwrap());
    assert_eq!(
        entry(&report.value, "hall", "HALL.md").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "claude-code", "CLAUDE.md alias").change,
        Change::Created
    );
    let block = fs::read_text(&root.join("HALL.md")).unwrap().unwrap();
    assert!(block.contains("# acme"));
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md")),
        "the enabled provider's alias must be a relative symlink to HALL.md"
    );
}

/// `sync` runs after every `git pull`. The second run must touch nothing,
/// or every run leaves a spurious modification in `git status`.
#[test]
fn a_second_sync_changes_nothing() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    let before = fs::read_bytes(&root.join("HALL.md")).unwrap().unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(
        report
            .value
            .entries
            .iter()
            .all(|e| e.change == Change::Unchanged),
        "expected every entry unchanged, got {:?}",
        report.value.entries
    );
    assert_eq!(
        fs::read_bytes(&root.join("HALL.md")).unwrap().unwrap(),
        before
    );
}

// -- repos -----------------------------------------------------------------

#[test]
fn a_declared_repo_is_cloned_bare_and_gets_its_default_branch_worktree() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "repo api", "worktree main").change,
        Change::Created
    );
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/main/README.md")).unwrap(),
        "seed\n"
    );
}

#[test]
fn the_managed_block_lists_every_declared_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    let block = fs::read_text(&root.join("HALL.md")).unwrap().unwrap();
    assert!(block.contains("`api`"));
    assert!(block.contains("`web`"));
}

/// The whole point of the warning channel: eight repos, one bad remote,
/// seven still set up and the process exits 1 rather than 2.
#[test]
fn an_unreachable_repo_becomes_a_warning_and_the_others_still_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let layout = Layout::at(root.clone());
    let mut repos = Manifest::read(&layout).unwrap().unwrap().repos().to_vec();
    repos.push(Repo::new(
        RepoName::new("ghost").unwrap(),
        root.join("no-such-origin").as_str(),
        BranchName::new("main").unwrap(),
    ));
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(!report.is_clean(), "a failed repo must not be a clean run");
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject, "repo ghost");
    assert_eq!(
        entry(&report.value, "repo ghost", "bare clone").change,
        Change::Failed
    );
    // The healthy repo still landed.
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert!(root.join(".ivar/repos/api/main/README.md").is_file());
}

/// git's own message for a non-empty clone target names the symptom, not
/// the cause. This says what is actually wrong and what to do about it.
#[test]
fn a_partial_clone_left_at_the_bare_path_is_named_rather_than_left_to_git() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let bare = root.join(".ivar/repos/api/.bare");
    fs::ensure_dir(&bare).unwrap();
    fs::write_text(&bare.join("leftover"), "junk").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "bare clone");
    assert_eq!(failed.change, Change::Failed);
    assert!(
        failed
            .detail
            .as_deref()
            .unwrap()
            .contains("is not a bare clone"),
        "was: {:?}",
        failed.detail
    );
}

/// The worktree twin of the case above. Both go through `occupied`, so this
/// is what keeps the shared helper honest about saying the *right* noun.
#[test]
fn something_else_at_the_worktree_path_is_named_rather_than_left_to_git() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let worktree = root.join(".ivar/repos/api/main");
    fs::ensure_dir(&worktree).unwrap();
    fs::write_text(&worktree.join("notes.md"), "mine").unwrap();
    let ctx = Ctx::new(root);

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "worktree");
    assert_eq!(failed.change, Change::Failed);
    assert!(
        failed
            .detail
            .as_deref()
            .unwrap()
            .contains("is not a worktree"),
        "was: {:?}",
        failed.detail
    );
}

/// `main` versus `master` is the commonest first-run mistake, and git's own
/// refusal names neither the manifest nor the branch that does exist.
#[test]
fn a_branch_the_repo_does_not_have_names_the_repos_default_instead() {
    let (_guard, root) = hall_with(&[("api", "master")]);
    // The origin is on `master`; declare `main` in the manifest instead.
    let layout = Layout::at(root.clone());
    let url = Manifest::read(&layout).unwrap().unwrap().repos()[0]
        .url()
        .to_owned();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            url,
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root);

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "worktree");
    assert_eq!(failed.change, Change::Failed);
    let detail = failed.detail.as_deref().unwrap();
    assert!(detail.contains("main"), "was: {detail}");
    assert!(
        detail.contains("master"),
        "the branch that DOES exist has to survive into the rendered sentence, \
             not sit in a field a per-item failure never renders: {detail}"
    );
    assert!(
        report.warnings.iter().any(|w| w.subject == "repo api"),
        "a named branch mismatch must still be a warning, not a silent skip"
    );
}
