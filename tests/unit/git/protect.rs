#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::git::exec::{add_worktree, clone_bare};
use crate::test_support::{seeded_repo, utf8_temp_dir};

// ---------------------------------------------------------------------------
// Default-branch protection
// ---------------------------------------------------------------------------

/// Run git in `cwd` without the scaffolding hook opt-out, so an installed
/// pre-commit hook is actually consulted. Returns success plus stderr.
fn git_unguarded(cwd: &Utf8Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn effective_config(worktree: &Utf8Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", worktree.as_str()])
        .args(["config", "--get", key])
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// A hall's bare repo already has worktrees by the time protection arrives.
/// Enabling `extensions.worktreeConfig` while `core.bare=true` is still
/// inherited makes every one of them answer "this operation must be run in a
/// work tree" — so the migration has to reach each worktree before the
/// extension is switched on.
#[test]
fn protection_migrates_existing_worktrees_without_breaking_them() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();

    let main_wt = dir.join("api/main");
    add_worktree(&bare, &main_wt, "main").unwrap();
    crate::test_support::git(&main_wt, &["branch", "feat/one"]);
    crate::test_support::git(&main_wt, &["branch", "feat/two"]);
    let one = dir.join("api/feat-one");
    let two = dir.join("api/feat-two");
    add_worktree(&bare, &one, "feat/one").unwrap();
    add_worktree(&bare, &two, "feat/two").unwrap();

    protect_default_branch(&bare, &main_wt, "main").unwrap();

    for wt in [&main_wt, &one, &two] {
        let (ok, stderr) = git_unguarded(wt, &["status", "--porcelain"]);
        assert!(ok, "{wt} became unusable: {stderr}");
    }

    // The worktrees added *after* protection are the ones that matter: a hall
    // adds one per promoted feature, so a migration that only repaired the
    // worktrees present at install time would break every feature from then on.
    crate::test_support::git(&main_wt, &["branch", "feat/later"]);
    let later = dir.join("api/feat-later");
    add_worktree(&bare, &later, "feat/later").unwrap();
    let (ok, stderr) = git_unguarded(&later, &["status", "--porcelain"]);
    assert!(
        ok,
        "a worktree added after protection is born broken: {stderr}"
    );

    // The bare repository is still bare.
    let output = std::process::Command::new("git")
        .args([
            "--git-dir",
            bare.as_str(),
            "rev-parse",
            "--is-bare-repository",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "true",
        "the bare repo must stay bare"
    );
}

/// The hook has to live somewhere a project's own hook manager will not
/// silently take over. Husky writes `core.hooksPath` into the shared config
/// from `pnpm install`, which is exactly what ivar's setup script runs.
#[test]
fn protection_survives_a_project_hook_manager_and_only_binds_the_default() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let main_wt = dir.join("api/main");
    add_worktree(&bare, &main_wt, "main").unwrap();
    crate::test_support::git(&main_wt, &["branch", "feat/x"]);
    let feat = dir.join("api/feat-x");
    add_worktree(&bare, &feat, "feat/x").unwrap();

    // Husky, arriving after the clone as it would from a setup script.
    let husky = dir.join("husky");
    crate::infra::fs::ensure_dir(&husky).unwrap();
    crate::test_support::git(&bare, &["config", "core.hooksPath", husky.as_str()]);

    protect_default_branch(&bare, &main_wt, "main").unwrap();

    let hooks = effective_config(&main_wt, "core.hooksPath").unwrap();
    assert_eq!(
        hooks,
        bare.join("ivar-hooks").as_str(),
        "the default worktree must resolve ivar's own hook dir, absolutely"
    );
    assert_eq!(
        effective_config(&feat, "core.hooksPath").as_deref(),
        Some(husky.as_str()),
        "a feature worktree keeps the project's hooks"
    );
}

#[test]
fn a_commit_on_the_default_branch_is_refused_and_a_feature_branch_is_not() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let main_wt = dir.join("api/main");
    add_worktree(&bare, &main_wt, "main").unwrap();
    crate::test_support::git(&main_wt, &["branch", "feat/x"]);
    let feat = dir.join("api/feat-x");
    add_worktree(&bare, &feat, "feat/x").unwrap();

    protect_default_branch(&bare, &main_wt, "main").unwrap();

    let (ok, stderr) = git_unguarded(&main_wt, &["commit", "--allow-empty", "-m", "nope"]);
    assert!(!ok, "a commit on the default branch must be refused");
    assert!(
        stderr.contains("main"),
        "the refusal must name the branch: {stderr}"
    );

    let (ok, stderr) = git_unguarded(&feat, &["commit", "--allow-empty", "-m", "fine"]);
    assert!(ok, "a feature branch must still commit: {stderr}");
}

/// `git rev-parse --abbrev-ref HEAD` answers the literal string `HEAD` on a
/// branch with no commits, which compares unequal to the branch name and lets
/// the very first commit through. `symbolic-ref` tells the truth.
#[test]
fn a_commit_on_an_unborn_default_branch_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = crate::test_support::empty_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();

    // An unborn branch has no worktree to add, so the origin checkout itself
    // stands in for one: same shape, no commits.
    protect_default_branch(&bare, &origin, "main").unwrap();

    std::fs::write(origin.join("a.txt"), "a\n").unwrap();
    crate::test_support::git(&origin, &["add", "a.txt"]);
    let (ok, stderr) = git_unguarded(&origin, &["commit", "-m", "first"]);
    assert!(
        !ok,
        "the first commit on an unborn default branch must be refused: {stderr}"
    );
}

#[test]
fn protection_is_idempotent_and_reports_what_it_did() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let main_wt = dir.join("api/main");
    add_worktree(&bare, &main_wt, "main").unwrap();

    let first = protect_default_branch(&bare, &main_wt, "main").unwrap();
    assert_eq!(first, Protection::Installed, "the first run installs");

    let hook = bare.join("ivar-hooks/pre-commit");
    let bytes = std::fs::read(hook.as_std_path()).unwrap();
    let mtime = std::fs::metadata(hook.as_std_path())
        .unwrap()
        .modified()
        .unwrap();

    let second = protect_default_branch(&bare, &main_wt, "main").unwrap();
    assert_eq!(
        second,
        Protection::AlreadyInstalled,
        "the second run is a no-op"
    );
    assert_eq!(
        std::fs::read(hook.as_std_path()).unwrap(),
        bytes,
        "unchanged hook bytes must not be rewritten"
    );
    assert_eq!(
        std::fs::metadata(hook.as_std_path())
            .unwrap()
            .modified()
            .unwrap(),
        mtime,
        "an idempotent run must not touch the file"
    );
}
