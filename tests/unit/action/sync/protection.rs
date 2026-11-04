use super::*;

/// Protection is not a separate command anyone has to remember: `sync` is the
/// one action that runs over every repo in the hall, so it is where an
/// existing hall acquires protection without being asked.
#[test]
fn sync_protects_the_default_branch_of_every_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "trunk")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    for (repo, branch) in [("api", "main"), ("web", "trunk")] {
        let worktree = root.join(format!(".ivar/repos/{repo}/{branch}"));
        let output = std::process::Command::new("git")
            .args(["-c", "user.name=ivar tests"])
            .args(["-c", "user.email=tests@ivar.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(["commit", "--allow-empty", "-m", "nope"])
            .current_dir(&worktree)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{repo}: a commit on {branch} must be refused"
        );
        assert!(
            stderr.contains(branch),
            "{repo}: the refusal must name {branch}: {stderr}"
        );
    }
}

/// A repo whose setup script installs husky must still end up protected, and
/// the ordering is the whole reason: protection runs after the setup script,
/// so whatever the script wrote to `core.hooksPath` is already there to be
/// overridden for the default worktree.
#[test]
fn protection_outlives_a_setup_script_that_rewrites_hooks_path() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    // Stand in for husky: write the shared config the way `pnpm install` would.
    let bare = root.join(".ivar/repos/api/.bare");
    let elsewhere = root.join("elsewhere");
    crate::infra::fs::ensure_dir(&elsewhere).unwrap();
    crate::test_support::git(&bare, &["config", "core.hooksPath", elsewhere.as_str()]);

    sync(&ctx, &SyncInput::default()).unwrap();

    let worktree = root.join(".ivar/repos/api/main");
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "--allow-empty", "-m", "nope"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a later hook manager must not disarm protection"
    );
}

#[test]
fn repeated_sync_preserves_prefixed_repo_refspec_and_config() {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();

    let origins = root.parent().unwrap().join("origins");
    let origin = seeded_repo(&origins.join("hall_origin"), "main");
    crate::test_support::git(&origin, &["branch", "repos/notes/main"]);

    let repo = Repo::new(
        RepoName::new("notes").unwrap(),
        origin.as_str(),
        BranchName::new("main").unwrap(),
    )
    .with_ref_prefix("repos/notes/");

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![repo],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    // First sync clones the prefixed repo.
    let report1 = sync(&ctx, &SyncInput::default()).unwrap();
    assert!(report1.is_clean());

    let bare = root.join(".ivar/repos/notes/.bare");
    let get_fetch = || {
        std::process::Command::new("git")
            .args([
                "--git-dir",
                bare.as_str(),
                "config",
                "--get",
                "remote.origin.fetch",
            ])
            .output()
            .unwrap()
    };
    let get_prefix = || {
        std::process::Command::new("git")
            .args([
                "--git-dir",
                bare.as_str(),
                "config",
                "--get",
                "ivar.refprefix",
            ])
            .output()
            .unwrap()
    };

    let fetch_out1 = get_fetch();
    assert!(fetch_out1.status.success());
    assert_eq!(
        String::from_utf8_lossy(&fetch_out1.stdout).trim(),
        "+refs/heads/repos/notes/*:refs/remotes/origin/*"
    );
    let prefix_out1 = get_prefix();
    assert!(prefix_out1.status.success());
    assert_eq!(
        String::from_utf8_lossy(&prefix_out1.stdout).trim(),
        "repos/notes/"
    );

    // Second sync: ensures ensure_remote_tracking preserves the custom prefix refspec
    let report2 = sync(&ctx, &SyncInput::default()).unwrap();
    assert!(report2.is_clean());

    let fetch_out2 = get_fetch();
    assert!(fetch_out2.status.success());
    assert_eq!(
        String::from_utf8_lossy(&fetch_out2.stdout).trim(),
        "+refs/heads/repos/notes/*:refs/remotes/origin/*"
    );
    let prefix_out2 = get_prefix();
    assert!(prefix_out2.status.success());
    assert_eq!(
        String::from_utf8_lossy(&prefix_out2.stdout).trim(),
        "repos/notes/"
    );

    let _ = guard;
}
