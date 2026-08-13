#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::git::exec;
use crate::test_support::{empty_repo, git, seeded_repo, utf8_temp_dir};

#[test]
fn target_state_recognises_a_worktree_repo_and_a_bare_one() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    exec::clone_bare(origin.as_str(), &bare).unwrap();

    assert_eq!(target_state(&origin).unwrap(), TargetState::Repository);
    assert_eq!(target_state(&bare).unwrap(), TargetState::Repository);
}

/// The distinction the three states exist for: a partial clone is not the
/// same as a clean slate, and only one of them is safe to clone into.
#[test]
fn target_state_tells_a_missing_path_from_one_holding_something_else() {
    let (_guard, dir) = utf8_temp_dir();

    assert_eq!(target_state(&dir).unwrap(), TargetState::Occupied);
    assert_eq!(
        target_state(&dir.join("does-not-exist")).unwrap(),
        TargetState::Absent
    );
}

#[test]
fn head_branch_strips_the_refs_heads_prefix() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "trunk");
    let bare = dir.join("api.bare");
    exec::clone_bare(origin.as_str(), &bare).unwrap();

    assert_eq!(head_branch(&bare).unwrap(), "trunk");
}

/// A bare clone of a repository with no commits is how a hall picks up a
/// repo created moments ago. Resolving `HEAD` would fail there with
/// "reference not found", which names the wrong problem — the branch is
/// known, it just has nothing on it yet.
#[test]
fn head_branch_answers_for_a_repository_with_no_commits() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = empty_repo(&dir.join("origin"), "main");

    assert_eq!(head_branch(&origin).unwrap(), "main");
}

#[test]
fn head_branch_on_something_that_is_not_a_repository_says_so() {
    let (_guard, dir) = utf8_temp_dir();

    let error = head_branch(&dir).expect_err("not a repository");

    match error {
        Error::NotARepository { path, detail } => {
            assert_eq!(path, dir);
            assert!(!detail.is_empty(), "libgit2 said nothing about why");
        }
        other => panic!("expected NotARepository, got {other:?}"),
    }
}

#[test]
fn worktree_git_dir_of_a_plain_repository_is_its_dot_git() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");

    let git_dir = worktree_git_dir(&origin).unwrap();

    assert!(git_dir.ends_with(".git/"), "was: {git_dir}");
}

#[test]
fn worktree_git_dir_of_a_linked_worktree_is_under_the_bare_repository() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    exec::clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api/main");
    exec::add_worktree(&bare, &worktree, "main").unwrap();

    let git_dir = worktree_git_dir(&worktree).unwrap();

    // libgit2 hands back a resolved path. On macOS a `TempDir` lives under
    // `/var/...`, whose real name is `/private/var/...`, so the comparison
    // has to be against the resolved form of the bare repository too — the
    // same trap `test_support::canonical_temp_dir` exists for.
    let bare = bare.canonicalize_utf8().unwrap();
    assert!(git_dir.starts_with(&bare), "{git_dir} is not under {bare}");
    assert!(git_dir.as_str().contains("worktrees"), "was: {git_dir}");
}

#[test]
fn is_ancestor_true_for_a_direct_parent() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    git(&repo, &["commit", "--allow-empty", "-m", "child"]);

    assert!(is_ancestor(&repo, "HEAD~1", "HEAD").unwrap());
}

#[test]
fn is_ancestor_true_for_a_distant_ancestor() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    for message in ["second", "third", "fourth", "fifth"] {
        git(&repo, &["commit", "--allow-empty", "-m", message]);
    }

    assert!(is_ancestor(&repo, "HEAD~4", "HEAD").unwrap());
}

/// Two branches that share a root but diverged afterward: neither is
/// reachable from the other by following parent links.
#[test]
fn is_ancestor_false_for_sibling_branches() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    git(&repo, &["checkout", "-b", "feature"]);
    git(&repo, &["commit", "--allow-empty", "-m", "on feature"]);
    git(&repo, &["checkout", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "on main"]);

    assert!(!is_ancestor(&repo, "feature", "main").unwrap());
    assert!(!is_ancestor(&repo, "main", "feature").unwrap());
}

/// `git2::Repository::graph_descendant_of` does not consider a commit its
/// own descendant, unlike the `git merge-base --is-ancestor` CLI it mirrors
/// otherwise — this pins that behaviour so a future switch to a different
/// git2 call surfaces the change instead of passing silently.
#[test]
fn is_ancestor_false_for_the_same_commit_on_both_sides() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");

    assert!(!is_ancestor(&repo, "HEAD", "HEAD").unwrap());
}

#[test]
fn is_ancestor_on_a_revision_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");

    let error = is_ancestor(&repo, "does-not-exist", "HEAD").expect_err("no such revision");

    match error {
        Error::Refused { detail, .. } => {
            assert!(!detail.is_empty(), "git said nothing about why");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}
