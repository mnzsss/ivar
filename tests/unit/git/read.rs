#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::{Utf8Path, Utf8PathBuf};

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

/// `git merge-base --is-ancestor` — the command `is_ancestor` mirrors —
/// considers a commit its own ancestor, exit 0. A branch sitting exactly at
/// its base's tip, with no commits of its own, resolves both sides to the
/// same commit; this pins that case to `true` so it is never mistaken for a
/// moved base.
#[test]
fn is_ancestor_true_for_the_same_commit_on_both_sides() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");

    assert!(is_ancestor(&repo, "HEAD", "HEAD").unwrap());
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

/// `divergence` reports the commits each side has that the other does not,
/// newest-first, for two branches that share a root and diverged.
#[test]
fn divergence_lists_the_commits_each_side_has_that_the_other_does_not() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    git(&repo, &["checkout", "-b", "feature"]);
    git(&repo, &["commit", "--allow-empty", "-m", "feature one"]);
    git(&repo, &["commit", "--allow-empty", "-m", "feature two"]);
    git(&repo, &["checkout", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "main one"]);

    let divergence = divergence(&repo, "feature", "main").unwrap();

    assert_eq!(divergence.ahead(), 2, "feature has two commits main lacks");
    assert_eq!(divergence.behind(), 1, "main has one commit feature lacks");
    let feature_subjects: Vec<_> = divergence
        .local_only
        .iter()
        .map(|commit| commit.subject.as_str())
        .collect();
    assert_eq!(
        feature_subjects,
        vec!["feature two", "feature one"],
        "local-only commits are newest-first"
    );
    assert_eq!(divergence.remote_only[0].subject, "main one");
    // The shas are real commit ids, not empty.
    for commit in divergence.local_only.iter().chain(&divergence.remote_only) {
        assert!(!commit.sha.is_empty());
    }
}

/// Two branches at the same tip have no divergence on either side.
#[test]
fn divergence_is_empty_when_both_sides_are_aligned() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");

    let divergence = divergence(&repo, "main", "HEAD").unwrap();

    assert!(divergence.local_only.is_empty());
    assert!(divergence.remote_only.is_empty());
    assert_eq!(divergence.ahead(), 0);
    assert_eq!(divergence.behind(), 0);
}

/// A revision that does not exist on either side is refused, not reported as
/// an empty divergence.
#[test]
fn divergence_on_a_revision_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");

    let error = divergence(&repo, "does-not-exist", "HEAD").expect_err("no such revision");

    match error {
        Error::Refused { .. } => {}
        other => panic!("expected Refused, got {other:?}"),
    }
}

// -- path_at_commit ---------------------------------------------------------

/// A real repository with `file.txt` committed twice, so there are two commits
/// whose trees differ at a known path. Answers `(worktree, first, second)`.
fn two_commits(dir: &Utf8Path) -> (Utf8PathBuf, String, String) {
    let repo = seeded_repo(&dir.join("repo"), "main");
    std::fs::write(repo.join("file.txt"), "one\n").unwrap();
    git(&repo, &["add", "file.txt"]);
    git(&repo, &["commit", "-m", "one"]);
    let first = exec::head_commit(&repo).unwrap();

    std::fs::write(repo.join("file.txt"), "two\n").unwrap();
    git(&repo, &["commit", "-am", "two"]);
    let second = exec::head_commit(&repo).unwrap();

    (repo, first, second)
}

#[test]
fn path_at_commit_reads_the_content_that_commit_holds() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, first, second) = two_commits(&dir);

    let before = path_at_commit(&repo, &first, Utf8Path::new("file.txt"))
        .unwrap()
        .unwrap();
    let after = path_at_commit(&repo, &second, Utf8Path::new("file.txt"))
        .unwrap()
        .unwrap();

    assert_eq!(before.sha256, hash::bytes(b"one\n"));
    assert_eq!(after.sha256, hash::bytes(b"two\n"));
    assert_eq!(before.mode, 0o100_644);
}

/// The hash has to compare equal to one taken over the worktree, because that
/// is the comparison run evidence is built out of. Git's own SHA-1 object id
/// would not.
#[test]
fn the_hash_matches_one_taken_over_the_same_file_in_the_worktree() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, _first, second) = two_commits(&dir);

    let from_commit = path_at_commit(&repo, &second, Utf8Path::new("file.txt"))
        .unwrap()
        .unwrap();

    assert_eq!(
        from_commit.sha256,
        hash::file(&repo.join("file.txt")).unwrap()
    );
}

/// "Nothing was there" is an answer, not a failure — it is exactly the
/// baseline an added file needs.
#[test]
fn a_path_the_commit_does_not_hold_answers_none() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, _first, second) = two_commits(&dir);

    assert_eq!(
        path_at_commit(&repo, &second, Utf8Path::new("never-existed.txt")).unwrap(),
        None
    );
}

/// A file added in the second commit did not exist in the first, which is what
/// makes it read as `Added` rather than `Modified`.
#[test]
fn a_file_added_later_is_absent_from_the_earlier_commit() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, first, second) = two_commits(&dir);
    std::fs::write(repo.join("new.txt"), "new\n").unwrap();
    git(&repo, &["add", "new.txt"]);
    git(&repo, &["commit", "-m", "add new"]);
    let third = exec::head_commit(&repo).unwrap();

    assert_eq!(
        path_at_commit(&repo, &first, Utf8Path::new("new.txt")).unwrap(),
        None
    );
    assert_eq!(
        path_at_commit(&repo, &second, Utf8Path::new("new.txt")).unwrap(),
        None
    );
    assert!(
        path_at_commit(&repo, &third, Utf8Path::new("new.txt"))
            .unwrap()
            .is_some()
    );
}

/// Flipping the executable bit changes no content hash. The mode is recorded
/// next to the hash precisely so that change is not invisible.
#[test]
fn the_filemode_distinguishes_an_executable_from_a_plain_file() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    std::fs::write(repo.join("run.sh"), "#!/bin/sh\n").unwrap();
    git(&repo, &["add", "run.sh"]);
    git(&repo, &["commit", "-m", "plain"]);
    let plain = exec::head_commit(&repo).unwrap();

    git(&repo, &["update-index", "--chmod=+x", "run.sh"]);
    git(&repo, &["commit", "-m", "executable"]);
    let executable = exec::head_commit(&repo).unwrap();

    let before = path_at_commit(&repo, &plain, Utf8Path::new("run.sh"))
        .unwrap()
        .unwrap();
    let after = path_at_commit(&repo, &executable, Utf8Path::new("run.sh"))
        .unwrap()
        .unwrap();

    assert_eq!(before.mode, 0o100_644);
    assert_eq!(after.mode, 0o100_755);
    assert_eq!(
        before.sha256, after.sha256,
        "the content did not change — only the mode did"
    );
}

/// A symlink's blob *is* its target, so a retargeted symlink hashes
/// differently. Hashing the file it points at would report no change at all.
#[test]
fn a_symlink_hashes_its_target_not_the_file_it_points_at() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    std::fs::write(repo.join("a.txt"), "same\n").unwrap();
    std::fs::write(repo.join("b.txt"), "same\n").unwrap();
    std::os::unix::fs::symlink("a.txt", repo.join("link")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "link to a"]);
    let to_a = exec::head_commit(&repo).unwrap();

    std::fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("b.txt", repo.join("link")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "link to b"]);
    let to_b = exec::head_commit(&repo).unwrap();

    let before = path_at_commit(&repo, &to_a, Utf8Path::new("link"))
        .unwrap()
        .unwrap();
    let after = path_at_commit(&repo, &to_b, Utf8Path::new("link"))
        .unwrap()
        .unwrap();

    assert_eq!(before.mode, 0o120_000, "a symlink's git filemode");
    assert_eq!(before.sha256, hash::bytes(b"a.txt"));
    assert_eq!(after.sha256, hash::bytes(b"b.txt"));
    assert_ne!(
        before.sha256, after.sha256,
        "the targets differ even though both files hold the same bytes"
    );
}

/// Binary content is bytes like any other — no encoding step to get wrong, and
/// no content leaving this module either way.
#[test]
fn a_binary_file_hashes_its_bytes() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    let bytes: Vec<u8> = (0u8..=255).collect();
    std::fs::write(repo.join("blob.bin"), &bytes).unwrap();
    git(&repo, &["add", "blob.bin"]);
    git(&repo, &["commit", "-m", "binary"]);
    let head = exec::head_commit(&repo).unwrap();

    let evidence = path_at_commit(&repo, &head, Utf8Path::new("blob.bin"))
        .unwrap()
        .unwrap();

    assert_eq!(evidence.sha256, hash::bytes(&bytes));
}

/// A directory is not a file a receipt describes, and neither is a submodule
/// gitlink. Both read as `None` rather than as an error, because the path sets
/// that feed this only ever name blobs.
#[test]
fn a_path_that_is_not_a_blob_answers_none() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    std::fs::create_dir(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "fn main() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "tree"]);
    let head = exec::head_commit(&repo).unwrap();

    assert_eq!(
        path_at_commit(&repo, &head, Utf8Path::new("src")).unwrap(),
        None
    );
    assert!(
        path_at_commit(&repo, &head, Utf8Path::new("src/lib.rs"))
            .unwrap()
            .is_some()
    );
}

/// The property the whole baseline rests on: the starting commit still exists
/// and still holds what it held, whatever the run did to HEAD afterwards.
#[test]
fn the_starting_commit_still_answers_after_a_commit_amend_reset_or_branch_switch() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, start, _second) = two_commits(&dir);
    let expected = hash::bytes(b"one\n");

    // commit
    std::fs::write(repo.join("file.txt"), "three\n").unwrap();
    git(&repo, &["commit", "-am", "three"]);
    assert_eq!(
        path_at_commit(&repo, &start, Utf8Path::new("file.txt"))
            .unwrap()
            .unwrap()
            .sha256,
        expected
    );

    // amend
    std::fs::write(repo.join("file.txt"), "four\n").unwrap();
    git(&repo, &["commit", "-a", "--amend", "-m", "four"]);
    assert_eq!(
        path_at_commit(&repo, &start, Utf8Path::new("file.txt"))
            .unwrap()
            .unwrap()
            .sha256,
        expected
    );

    // reset
    git(&repo, &["reset", "--hard", &start]);
    assert_eq!(
        path_at_commit(&repo, &start, Utf8Path::new("file.txt"))
            .unwrap()
            .unwrap()
            .sha256,
        expected
    );

    // branch switch
    git(&repo, &["checkout", "-b", "other"]);
    std::fs::write(repo.join("file.txt"), "five\n").unwrap();
    git(&repo, &["commit", "-am", "five"]);
    assert_eq!(
        path_at_commit(&repo, &start, Utf8Path::new("file.txt"))
            .unwrap()
            .unwrap()
            .sha256,
        expected
    );
}

/// A rebase rewrites every commit it moves, and the *starting* commit is
/// therefore no longer on the branch. It is still reachable by id, which is
/// the only reason recording an id rather than a ref works.
#[test]
fn the_starting_commit_still_answers_after_a_rebase_rewrites_the_branch() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    std::fs::write(repo.join("file.txt"), "one\n").unwrap();
    git(&repo, &["add", "file.txt"]);
    git(&repo, &["commit", "-m", "one"]);
    let base = exec::head_commit(&repo).unwrap();

    git(&repo, &["checkout", "-b", "topic"]);
    std::fs::write(repo.join("topic.txt"), "topic\n").unwrap();
    git(&repo, &["add", "topic.txt"]);
    git(&repo, &["commit", "-m", "topic"]);
    let start = exec::head_commit(&repo).unwrap();

    git(&repo, &["checkout", "main"]);
    std::fs::write(repo.join("main.txt"), "main\n").unwrap();
    git(&repo, &["add", "main.txt"]);
    git(&repo, &["commit", "-m", "main moves"]);

    git(&repo, &["checkout", "topic"]);
    git(&repo, &["rebase", "main"]);

    assert_ne!(
        exec::head_commit(&repo).unwrap(),
        start,
        "the rebase must have rewritten the branch for this test to mean anything"
    );
    assert_eq!(
        path_at_commit(&repo, &start, Utf8Path::new("topic.txt"))
            .unwrap()
            .unwrap()
            .sha256,
        hash::bytes(b"topic\n")
    );
    assert_eq!(
        path_at_commit(&repo, &base, Utf8Path::new("topic.txt")).unwrap(),
        None
    );
}

/// A revision that does not exist is git's own refusal, never `Ok(None)` —
/// which would read as "nothing was there" and hide that the baseline itself
/// is gone.
#[test]
fn a_commit_that_does_not_exist_is_a_refusal_not_an_absent_path() {
    let (_guard, dir) = utf8_temp_dir();
    let (repo, _first, _second) = two_commits(&dir);

    assert!(
        path_at_commit(
            &repo,
            "0000000000000000000000000000000000000000",
            Utf8Path::new("file.txt")
        )
        .is_err()
    );
}

/// Paths with spaces and non-ASCII bytes are looked up as given. Nothing here
/// quotes or unquotes, so there is no escaping dialect to get wrong.
#[test]
fn a_path_with_spaces_and_non_ascii_bytes_is_read_as_given() {
    let (_guard, dir) = utf8_temp_dir();
    let repo = seeded_repo(&dir.join("repo"), "main");
    let name = "a directory/naïve file.txt";
    std::fs::create_dir(repo.join("a directory")).unwrap();
    std::fs::write(repo.join(name), "held\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "awkward"]);
    let head = exec::head_commit(&repo).unwrap();

    assert_eq!(
        path_at_commit(&repo, &head, Utf8Path::new(name))
            .unwrap()
            .unwrap()
            .sha256,
        hash::bytes(b"held\n")
    );
}
