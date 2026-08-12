#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::{seeded_repo, utf8_temp_dir};

#[test]
fn clone_bare_produces_a_bare_repository() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");

    clone_bare(origin.as_str(), &bare).unwrap();

    // A bare clone has its object database at the top level, not under
    // `.git/` — that is what makes it a valid `--git-dir`.
    assert!(bare.join("HEAD").is_file());
    assert!(bare.join("objects").is_dir());
    assert!(!bare.join(".git").exists());
}

#[test]
fn a_failing_clone_is_refused_with_the_invocation_and_gits_diagnostic() {
    let (_guard, dir) = utf8_temp_dir();

    let error =
        clone_bare(dir.join("nowhere").as_str(), &dir.join("dest")).expect_err("nothing to clone");

    match error {
        Error::Refused { command, detail } => {
            assert!(command.starts_with("git clone --bare"), "was: {command}");
            assert!(!detail.is_empty(), "git said nothing about why");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}

#[test]
fn add_worktree_checks_out_the_branchs_content() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api/main");

    add_worktree(&bare, &worktree, "main").unwrap();

    assert_eq!(
        std::fs::read_to_string(worktree.join("README.md")).unwrap(),
        "seed\n"
    );
}

#[test]
fn adding_a_worktree_on_a_branch_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();

    let error =
        add_worktree(&bare, &dir.join("wt"), "no-such-branch").expect_err("branch does not exist");

    assert!(matches!(error, Error::Refused { .. }));
}

/// A worktree path that already holds something is git's call to refuse,
/// not this module's to pre-empt. Duplicating the check here would mean two
/// answers to one question, and git's is the one that is always right.
#[test]
fn adding_a_worktree_over_an_occupied_path_is_refused_by_git() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let occupied = dir.join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("something"), "here").unwrap();

    let error = add_worktree(&bare, &occupied, "main").expect_err("path is occupied");

    assert!(matches!(error, Error::Refused { .. }));
}

// -- remote-tracking refs -----------------------------------------------------
//
// `git clone --bare` writes no `remote.origin.fetch`, so nothing ever lands in
// `refs/remotes/origin/*`. Everything that reads a tracking ref then breaks in
// a hall and nowhere else: `git push --force-with-lease` refuses with "stale
// info", and `branch@{upstream}` does not resolve.

#[test]
fn clone_bare_configures_the_remote_tracking_refspec() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");

    clone_bare(origin.as_str(), &bare).unwrap();

    assert_eq!(
        config_value(&bare, "remote.origin.fetch"),
        Some("+refs/heads/*:refs/remotes/origin/*".to_owned())
    );
}

#[test]
fn a_bare_clone_carries_remote_tracking_refs() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");

    clone_bare(origin.as_str(), &bare).unwrap();

    assert!(
        ref_exists(&bare, "refs/remotes/origin/main"),
        "no tracking ref: --force-with-lease has nothing to lease against"
    );
    // The bare's own branches are still where a worktree expects them.
    assert!(ref_exists(&bare, "refs/heads/main"));
}

/// The layout ivar actually ships: a worktree off a bare clone, pushing back
/// with a lease. This is the regression the refspec exists for.
#[test]
fn force_with_lease_from_a_worktree_is_not_stale() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    // A push needs a remote that is not checked out anywhere.
    crate::test_support::git(&origin, &["config", "receive.denyCurrentBranch", "ignore"]);
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api/main");
    add_worktree(&bare, &worktree, "main").unwrap();
    std::fs::write(worktree.join("README.md"), "changed\n").unwrap();
    crate::test_support::git(&worktree, &["commit", "-am", "change"]);

    fetch(&bare).unwrap();
    let pushed = std::process::Command::new("git")
        .args(["push", "--force-with-lease", "origin", "main"])
        .current_dir(&worktree)
        .output()
        .unwrap();

    assert!(
        pushed.status.success(),
        "push refused: {}",
        String::from_utf8_lossy(&pushed.stderr)
    );
}

/// A push goes to a URL, which git records nothing about — so the push
/// records it itself. The tracking ref is then what ivar last sent, which is
/// exactly what a lease wants to know.
#[test]
fn a_push_records_what_it_sent_in_the_tracking_ref() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    crate::test_support::git(&bare, &["branch", "feat/x", "main"]);
    let worktree = dir.join("api/feat-x");
    add_worktree(&bare, &worktree, "feat/x").unwrap();
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    crate::test_support::git(&worktree, &["add", "work.md"]);
    crate::test_support::git(&worktree, &["commit", "-m", "work"]);

    push(&bare, origin.as_str(), "feat/x", "refs/heads/feat/x").unwrap();

    assert_eq!(
        ref_value(&bare, "refs/remotes/origin/feat/x"),
        ref_value(&bare, "refs/heads/feat/x"),
        "the push landed but left no record of itself"
    );
}

/// The regression the recording exists for. `deliver` pushes from the bare by
/// URL; the human then rewrites the commit in their worktree and leases the
/// force-push. Nothing fetches in between — nothing ever does — so the lease
/// has only what the push recorded to check against.
#[test]
fn force_with_lease_after_a_push_by_url_is_not_stale() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    crate::test_support::git(&origin, &["config", "receive.denyCurrentBranch", "ignore"]);
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api/main");
    add_worktree(&bare, &worktree, "main").unwrap();
    std::fs::write(worktree.join("README.md"), "changed\n").unwrap();
    crate::test_support::git(&worktree, &["commit", "-am", "change"]);

    push(&bare, origin.as_str(), "main", "refs/heads/main").unwrap();
    crate::test_support::git(&worktree, &["commit", "--amend", "-m", "changed again"]);

    let pushed = std::process::Command::new("git")
        .args(["push", "--force-with-lease", "origin", "main"])
        .current_dir(&worktree)
        .output()
        .unwrap();

    assert!(
        pushed.status.success(),
        "push refused: {}",
        String::from_utf8_lossy(&pushed.stderr)
    );
}

/// A push aimed somewhere other than origin says nothing about origin, so it
/// records nothing. Guessing here would put a commit the remote does not have
/// behind a ref that claims it does.
#[test]
fn a_push_to_a_url_that_is_not_origin_records_nothing() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let elsewhere = seeded_repo(&dir.join("elsewhere"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    crate::test_support::git(&bare, &["branch", "feat/x", "main"]);

    push(&bare, elsewhere.as_str(), "feat/x", "refs/heads/feat/x").unwrap();

    assert!(!ref_exists(&bare, "refs/remotes/origin/feat/x"));
}

/// Halls cloned before the refspec existed must be repairable in place —
/// re-cloning is not an option once feature branches live in the bare.
#[test]
fn ensure_remote_tracking_repairs_a_bare_cloned_without_the_refspec() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    // Put the bare back the way an older ivar left it.
    crate::test_support::git(&bare, &["config", "--unset", "remote.origin.fetch"]);
    assert_eq!(config_value(&bare, "remote.origin.fetch"), None);

    ensure_remote_tracking(&bare).unwrap();

    assert_eq!(
        config_value(&bare, "remote.origin.fetch"),
        Some("+refs/heads/*:refs/remotes/origin/*".to_owned())
    );
    fetch(&bare).unwrap();
    assert!(ref_exists(&bare, "refs/remotes/origin/main"));
}

#[test]
fn ensure_remote_tracking_is_idempotent_and_leaves_one_refspec() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();

    ensure_remote_tracking(&bare).unwrap();
    ensure_remote_tracking(&bare).unwrap();

    let all = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str()])
        .args(["config", "--get-all", "remote.origin.fetch"])
        .output()
        .unwrap();
    let lines = String::from_utf8_lossy(&all.stdout);
    assert_eq!(
        lines.lines().count(),
        1,
        "a repeated sync must not stack refspecs: {lines}"
    );
}

fn config_value(git_dir: &Utf8Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["--git-dir", git_dir.as_str()])
        .args(["config", "--get", key])
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The commit a ref points at, or `None` when the ref is not there.
fn ref_value(git_dir: &Utf8Path, refname: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["--git-dir", git_dir.as_str()])
        .args(["rev-parse", "--verify", "--quiet", refname])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !stdout.is_empty()).then_some(stdout)
}

fn ref_exists(git_dir: &Utf8Path, refname: &str) -> bool {
    std::process::Command::new("git")
        .args(["--git-dir", git_dir.as_str()])
        .args(["show-ref", "--verify", "--quiet", refname])
        .status()
        .unwrap()
        .success()
}
