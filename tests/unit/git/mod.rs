#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::error::{Failure, Status};
use crate::infra::proc;
use crate::test_support::{empty_repo, git, seeded_repo, utf8_temp_dir};

// -- System routes each operation to a backend that answers ---------------

#[test]
fn system_clones_a_bare_repository_and_reads_its_head_branch() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");

    System.clone_bare(origin.as_str(), &bare).unwrap();

    assert_eq!(System.target_state(&bare).unwrap(), TargetState::Repository);
    assert_eq!(System.head_branch(&bare).unwrap(), "main");
}

#[test]
fn system_adds_a_worktree_on_an_existing_branch() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();

    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    assert!(worktree.join("README.md").is_file());
    assert_eq!(
        System.target_state(&worktree).unwrap(),
        TargetState::Repository
    );
}

/// The admin dir of a linked worktree is `<bare>/worktrees/<name>/`, not
/// the bare repository. Bookkeeping written there dies with the worktree,
/// which is the property `sync`'s setup-script receipt depends on.
#[test]
fn a_worktrees_git_dir_is_its_own_and_not_the_bare_repository() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    let git_dir = System.worktree_git_dir(&worktree).unwrap();

    assert!(
        git_dir.as_str().contains("worktrees"),
        "expected a linked-worktree admin dir, got {git_dir}"
    );
    assert_ne!(git_dir, bare);
}

// -- fetch -----------------------------------------------------------------

/// Fetching updates the bare clone with commits the origin gained since
/// the clone. A bare clone has no worktree, so the update is only visible
/// to git — this asserts the fetch reached the object database.
#[test]
fn fetch_pulls_new_commits_from_the_origin_into_the_bare_clone() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();

    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    System.fetch(&bare).unwrap();

    // A bare clone has no `refs/remotes/*` — it copies `refs/heads/*`
    // straight over, so the fetched branch is `main`, not `origin/main`.
    let status = std::process::Command::new("git")
        .args(["--git-dir", bare.as_str(), "rev-parse", "main"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "main did not update: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

/// An up-to-date clone still fetches — `--quiet` makes it a no-op, but
/// the operation succeeds. The caller reports "up to date" through the
/// `Ok`, not through an error.
#[test]
fn fetch_succeeds_when_there_is_nothing_to_fetch() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();

    System.fetch(&bare).unwrap();
}

// -- fetch_branch + fast_forward (the pull refresh) -----------------------

/// The fetch-and-fast-forward `repo pull` runs: fetch lands in
/// `FETCH_HEAD` without touching the checked-out branch, then the
/// fast-forward advances the worktree's files and the shared branch ref.
#[test]
fn fetch_branch_then_fast_forward_updates_the_default_worktree() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    System.fetch_branch(&worktree, "main").unwrap();
    System.fast_forward(&worktree).unwrap();

    assert_eq!(
        std::fs::read_to_string(worktree.join("CHANGELOG.md")).unwrap(),
        "v1\n",
        "the worktree's files must catch up to the fetched tip"
    );
}

/// The "skipped" case: a default branch that diverged cannot be
/// fast-forwarded, and that is reported as a refusal — never a silent
/// clobber of the local commit.
#[test]
fn fast_forward_refuses_a_diverged_worktree() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    // The worktree gains a local commit while the origin moves elsewhere.
    git(&worktree, &["commit", "--allow-empty", "-m", "local drift"]);
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    System.fetch_branch(&worktree, "main").unwrap();
    let error = System.fast_forward(&worktree).expect_err("diverged");

    assert!(matches!(error, Error::Refused { .. }));
}

// -- remove_worktree -------------------------------------------------------

#[test]
fn remove_worktree_takes_a_worktree_down() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();
    assert!(worktree.join("README.md").is_file());

    System.remove_worktree(&bare, &worktree).unwrap();

    assert_eq!(System.target_state(&worktree).unwrap(), TargetState::Absent);
}

#[test]
fn remove_worktree_refuses_a_path_git_does_not_manage() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    // A hand-made directory at where a worktree should be: not a git
    // worktree, so `git worktree remove` refuses — the best-effort
    // "step failed, continue" path deregister relies on.
    let stray = dir.join("stray");
    std::fs::create_dir_all(&stray).unwrap();

    let error = System
        .remove_worktree(&bare, &stray)
        .expect_err("not a registered worktree");

    assert!(matches!(error, Error::Refused { .. }));
}

// -- list_branches ----------------------------------------------------------

#[test]
fn list_branches_returns_local_branch_names_without_the_prefix() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    git(&origin, &["branch", "dev"]);
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();

    let branches = System.list_branches(&bare).unwrap();

    // Sorted lexically, whatever order git2 happened to iterate in.
    assert_eq!(branches, vec!["dev".to_owned(), "main".to_owned()]);
}

/// An unborn default branch has no ref for `list_branches` to find —
/// that is not an error, it is the truth about a repo created moments ago.
#[test]
fn list_branches_answers_empty_for_a_repository_with_no_commits() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = empty_repo(&dir.join("origin"), "main");

    let branches = System.list_branches(&origin).unwrap();

    assert!(branches.is_empty());
}

#[test]
fn list_branches_on_something_that_is_not_a_repository_says_so() {
    let (_guard, dir) = utf8_temp_dir();

    let error = System.list_branches(&dir).expect_err("not a repository");

    assert!(matches!(error, Error::NotARepository { .. }));
}

/// `target_state` answers about the path itself. If it walked up, every
/// empty directory inside a hall that is itself a git repo would claim to
/// be a materialised worktree.
#[test]
fn target_state_does_not_walk_up_to_a_parent_repository() {
    let (_guard, dir) = utf8_temp_dir();
    seeded_repo(&dir, "main");
    let child = dir.join("not-a-repo");
    std::fs::create_dir_all(&child).unwrap();

    assert_eq!(System.target_state(&dir).unwrap(), TargetState::Repository);
    assert_eq!(System.target_state(&child).unwrap(), TargetState::Occupied);
}

#[test]
fn cloning_a_url_that_is_not_a_repository_is_refused_with_gits_own_message() {
    let (_guard, dir) = utf8_temp_dir();

    let error = System
        .clone_bare(dir.join("nowhere").as_str(), &dir.join("dest"))
        .expect_err("nothing to clone");

    assert!(matches!(error, Error::Refused { .. }));
}

// -- worktree_dirty --------------------------------------------------------

#[test]
fn worktree_dirty_reports_a_clean_worktree_as_clean() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    assert!(!System.worktree_dirty(&worktree).unwrap());
}

#[test]
fn worktree_dirty_sees_untracked_files_as_dirty() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();

    std::fs::write(worktree.join("notes.md"), "mine\n").unwrap();

    assert!(System.worktree_dirty(&worktree).unwrap());
}

// -- changed_paths ---------------------------------------------------------

/// A worktree on `main`, cloned from a seeded origin — the four tests below
/// differ only in what they then write into it, so the setup is stated once.
fn worktree_on_main() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api-main");
    System.add_worktree(&bare, &worktree, "main").unwrap();
    (guard, worktree)
}

/// The write-contract audit asks *which files*, not *whether any*. A tracked
/// edit and a file the executor created are both writes; only the first is
/// visible to `git diff`, which is why this reads `status` and asks for
/// untracked files explicitly.
#[test]
fn changed_paths_names_tracked_edits_and_untracked_files_alike() {
    let (_guard, worktree) = worktree_on_main();

    std::fs::write(worktree.join("README.md"), "edited\n").unwrap();
    std::fs::create_dir_all(worktree.join("src/deep")).unwrap();
    std::fs::write(worktree.join("src/deep/new.rs"), "fn main() {}\n").unwrap();

    let changed = System.changed_paths(&worktree).unwrap();

    assert!(
        changed.iter().any(|path| path == "README.md"),
        "{changed:?}"
    );
    assert!(
        changed.iter().any(|path| path == "src/deep/new.rs"),
        "an untracked file inside a new directory must be named, not collapsed \
         into the directory: {changed:?}"
    );
}

#[test]
fn changed_paths_is_empty_for_a_clean_worktree() {
    let (_guard, worktree) = worktree_on_main();

    assert!(System.changed_paths(&worktree).unwrap().is_empty());
}

/// A path with a space in it is exactly the path the default porcelain format
/// quotes, and a quoted path is one the audit would compare against a contract
/// with the quotes still on it — always allowed, always wrong.
#[test]
fn changed_paths_does_not_quote_a_path_with_a_space_in_it() {
    let (_guard, worktree) = worktree_on_main();

    std::fs::write(worktree.join("release notes.md"), "hi\n").unwrap();

    let changed = System.changed_paths(&worktree).unwrap();

    assert!(
        changed.iter().any(|path| path == "release notes.md"),
        "{changed:?}"
    );
}

/// The same quoting hazard, one byte worse: a path holding a literal newline.
/// NUL-separated records need no escaping, so the dirty half of the
/// existing-work query must hand the path back verbatim for a contract to
/// match.
#[test]
fn changed_paths_does_not_quote_a_path_with_a_newline_in_it() {
    let (_guard, worktree) = worktree_on_main();

    std::fs::write(worktree.join("notes\nwith a newline.md"), "hi\n").unwrap();

    let changed = System.changed_paths(&worktree).unwrap();

    assert!(
        changed
            .iter()
            .any(|path| path.as_str() == "notes\nwith a newline.md"),
        "{changed:?}"
    );
}

/// The committed half of the existing-work query has the same NUL contract:
/// `git diff --name-only` in its default form quotes a path holding a space
/// or a newline, and a quoted path never matches a write contract. `-z`
/// returns it raw.
#[test]
fn paths_committed_since_returns_paths_with_spaces_and_newlines_verbatim() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);
    let worktree = dir.join("feat-x");
    System.add_worktree(&bare, &worktree, "feat/x").unwrap();

    let odd_name = "release notes\n2024.md";
    std::fs::write(worktree.join(odd_name), "hi\n").unwrap();
    git(&worktree, &["add", "-A"]);
    git(&worktree, &["commit", "-m", "odd name"]);

    let committed = System.paths_committed_since(&worktree, "main").unwrap();

    assert!(
        committed.iter().any(|path| path.as_str() == odd_name),
        "the committed path must survive verbatim: {committed:?}"
    );
}

/// Both ends of a rename are writes: the file at the old path is gone.
#[test]
fn changed_paths_names_both_ends_of_a_rename() {
    let (_guard, worktree) = worktree_on_main();

    git(&worktree, &["mv", "README.md", "READYOU.md"]);

    let changed = System.changed_paths(&worktree).unwrap();

    assert!(
        changed.iter().any(|path| path == "READYOU.md"),
        "{changed:?}"
    );
    assert!(
        changed.iter().any(|path| path == "README.md"),
        "{changed:?}"
    );
}

// -- commits_ahead ---------------------------------------------------------

/// A feature branch created off `main` with one new commit is one ahead of
/// `main`; the reverse direction is zero.
#[test]
fn commits_ahead_counts_commits_beyond_the_base() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);
    let worktree = dir.join("feat-x");
    System.add_worktree(&bare, &worktree, "feat/x").unwrap();

    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    assert_eq!(System.commits_ahead(&bare, "main", "feat/x").unwrap(), 1);
    assert_eq!(System.commits_ahead(&bare, "feat/x", "main").unwrap(), 0);
}

#[test]
fn commits_ahead_is_zero_for_a_branch_at_the_base() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);

    assert_eq!(System.commits_ahead(&bare, "main", "feat/x").unwrap(), 0);
}

// -- remote_branch_tip -----------------------------------------------------

/// The branch the remote has never seen is absent; the same branch after a
/// push reads back at its tip.
///
/// This asks the remote even though a push now records itself locally, and it
/// has to: a tracking ref is only ever what *ivar* last sent, and anyone else
/// may have pushed since.
#[test]
fn remote_branch_tip_is_none_until_the_branch_is_pushed() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);

    assert_eq!(
        System
            .remote_branch_tip(&bare, origin.as_str(), "feat/x")
            .unwrap(),
        None
    );

    System
        .push(&bare, origin.as_str(), "feat/x", "refs/heads/feat/x")
        .unwrap();

    let tip = System
        .remote_branch_tip(&bare, origin.as_str(), "feat/x")
        .unwrap()
        .expect("the remote holds the branch it was just pushed");
    let local = {
        let output = std::process::Command::new("git")
            .args(["--git-dir", bare.as_str(), "rev-parse", "feat/x"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    assert_eq!(tip, local);
}

#[test]
fn remote_branch_tip_of_a_remote_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();

    let error = System
        .remote_branch_tip(&bare, dir.join("no-such-remote").as_str(), "feat/x")
        .expect_err("there is no remote to ask");

    assert!(matches!(error, Error::Refused { .. }));
}

// -- push ------------------------------------------------------------------

#[test]
fn push_creates_the_branch_on_the_remote() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);
    let worktree = dir.join("feat-x");
    System.add_worktree(&bare, &worktree, "feat/x").unwrap();

    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
    let tip = {
        let status = std::process::Command::new("git")
            .args(["--git-dir", bare.as_str(), "rev-parse", "feat/x"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&status.stdout).trim().to_owned()
    };

    System
        .push(&bare, origin.as_str(), "feat/x", "refs/heads/feat/x")
        .unwrap();

    let status = std::process::Command::new("git")
        .args(["ls-remote", origin.as_str(), "refs/heads/feat/x"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "ls-remote failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        stdout.contains(&tip),
        "the remote must hold the pushed tip: {stdout}"
    );
}

#[test]
fn push_to_a_remote_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);

    let error = System
        .push(
            &bare,
            dir.join("no-such-remote").as_str(),
            "feat/x",
            "refs/heads/feat/x",
        )
        .expect_err("nothing to push to");

    assert!(matches!(error, Error::Refused { .. }));
}

#[test]
fn head_branch_on_a_plain_directory_is_not_a_repository() {
    let (_guard, dir) = utf8_temp_dir();

    let error = System.head_branch(&dir).expect_err("not a repository");

    assert!(matches!(error, Error::NotARepository { .. }));
}

// -- Error -> Failure ------------------------------------------------------

/// A refused command is `Failed`, not `Blocked`: git got as far as trying,
/// and a half-finished clone leaves a directory behind. Claiming nothing
/// happened would be a lie the caller acts on.
#[test]
fn a_refused_command_is_a_failed_failure_carrying_gits_own_diagnostic() {
    let failure: Failure = Error::Refused {
        command: "git clone --bare url dest".to_owned(),
        detail: "repository 'url' does not exist".to_owned(),
    }
    .into();

    assert_eq!(failure.status, Status::Failed);
    assert_eq!(failure.code, "git.command_failed");
    assert_eq!(
        failure.actual.as_deref(),
        Some("repository 'url' does not exist")
    );
}

/// The one refusal whose way out is a specific pair of commands rather than
/// "read git's message". Left as the generic `git.command_failed`, the
/// envelope hands over a wall of git text and a fix action telling the user to
/// run the command that just failed — which will fail again the same way.
///
/// Reached whenever `ivar` writes history on a machine with no identity
/// configured: the squash commit and the no-ff merge in `exec.rs` deliberately
/// do not force one, because a commit landing in a user's repository must
/// carry that user's authorship.
#[test]
fn a_missing_git_identity_names_the_config_commands() {
    let failure: Failure = Error::Refused {
        command: "git commit -m Squashed child work".to_owned(),
        detail: "Author identity unknown\n\n*** Please tell me who you are.\n\nRun\n\n  \
                 git config --global user.email \"you@example.com\"\n  git config \
                 --global user.name \"Your Name\"\n\nto set your account's default \
                 identity.\nOmit --global to set the identity only in this \
                 repository.\n\nfatal: empty ident name (for <runner@host>) not allowed"
            .to_owned(),
    }
    .into();

    assert_eq!(failure.code, "git.identity_missing");
    assert_eq!(failure.status, Status::Blocked);
    let fix = &failure.fix_actions[0];
    assert!(
        fix.command
            .as_deref()
            .is_some_and(|command| command.contains("user.email")),
        "the fix must carry the command that sets an identity, got: {:?}",
        fix.command
    );
}

/// A refusal that merely mentions one config key is not the identity case —
/// the generic conversion still owns everything git says no to.
#[test]
fn an_unrelated_refusal_stays_the_generic_command_failure() {
    let failure: Failure = Error::Refused {
        command: "git config --get user.email".to_owned(),
        detail: "error: key does not contain a section: user".to_owned(),
    }
    .into();

    assert_eq!(failure.code, "git.command_failed");
}

#[test]
fn a_missing_repository_is_blocked_and_points_at_sync() {
    let failure: Failure = Error::NotARepository {
        path: Utf8PathBuf::from("/hall/.ivar/repos/api/.bare"),
        detail: "could not find repository".to_owned(),
    }
    .into();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "git.not_a_repository");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
}

#[test]
fn a_detached_head_needs_a_human() {
    let failure: Failure = Error::DetachedHead {
        path: Utf8PathBuf::from("/hall/.ivar/repos/api/main"),
    }
    .into();

    assert_eq!(failure.code, "git.detached_head");
    assert!(!failure.fix_actions[0].safe);
}

#[test]
fn a_spawn_error_delegates_its_failure_conversion() {
    let spawn = proc::capture(&proc::Command::new("ivar-no-such-program-exists-anywhere"))
        .expect_err("no binary");

    let failure: Failure = Error::Spawn(spawn).into();

    assert_eq!(failure.code, "proc.spawn_failed");
}

// -- temporary local-integration primitives --------------------------------

/// A bare clone with `parent` and `child` branches, and a worktree on each —
/// the shape local integration stages against.
fn integration_repo() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "parent"]);
    git(&bare, &["branch", "child"]);
    let parent_wt = dir.join("parent");
    System.add_worktree(&bare, &parent_wt, "parent").unwrap();
    let child_wt = dir.join("child");
    System.add_worktree(&bare, &child_wt, "child").unwrap();
    std::fs::write(child_wt.join("work.md"), "child work\n").unwrap();
    git(&child_wt, &["add", "work.md"]);
    git(&child_wt, &["commit", "-m", "child work"]);
    (guard, bare, parent_wt)
}

fn rev_parse(git_dir: &Utf8Path, rev: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["--git-dir", git_dir.as_str(), "rev-parse", rev])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The number of parent commits of `rev` — 0 for a root, 1 for an ordinary
/// commit, 2 for a merge commit.
fn parent_count(git_dir: &Utf8Path, rev: &str) -> usize {
    let output = std::process::Command::new("git")
        .args([
            "--git-dir",
            git_dir.as_str(),
            "rev-list",
            "--parents",
            "-n",
            "1",
            rev,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .count()
        - 1
}

#[test]
fn revision_commit_resolves_a_branch_to_its_tip() {
    let (guard, bare, _) = integration_repo();
    let _ = guard;
    let tip = rev_parse(&bare, "child");
    assert_eq!(System.revision_commit(&bare, "child").unwrap(), tip);
    assert!(System.revision_commit(&bare, "no-such-branch").is_err());
}

#[test]
fn add_detached_worktree_checks_out_the_revision_on_no_branch() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    let child_tip = rev_parse(&bare, "child");

    let candidate = parent_wt.join("candidate");
    System
        .add_detached_worktree(&bare, &candidate, &child_tip)
        .unwrap();

    assert!(candidate.join("work.md").is_file());
    assert_eq!(System.head_commit(&candidate).unwrap(), child_tip);
    // Detached: no branch is checked out.
    let output = std::process::Command::new("git")
        .args(["-C", candidate.as_str(), "branch", "--show-current"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "");
}

#[test]
fn create_and_delete_branch_are_a_temporary_lifecycle() {
    let (guard, bare, _) = integration_repo();
    let _ = guard;
    let child_tip = rev_parse(&bare, "child");
    let temp = "ivar-integrate/child/api";

    System.create_branch(&bare, temp, &child_tip).unwrap();
    assert_eq!(rev_parse(&bare, temp), child_tip);

    System.delete_branch(&bare, temp).unwrap();
    assert!(System.revision_commit(&bare, temp).is_err());
}

#[test]
fn merge_no_ff_produces_a_two_parent_merge_commit() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    let parent_before = rev_parse(&bare, "parent");
    let child_tip = rev_parse(&bare, "child");

    System.merge_no_ff(&parent_wt, "child").unwrap();

    let parent_after = rev_parse(&bare, "parent");
    assert_ne!(parent_after, parent_before);
    assert_eq!(parent_count(&bare, "parent"), 2);
    assert!(parent_wt.join("work.md").is_file());
    assert_eq!(
        child_tip,
        rev_parse(&bare, "child"),
        "the child never moves"
    );
}

#[test]
fn squash_merge_produces_a_single_parent_commit() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    let parent_before = rev_parse(&bare, "parent");

    System
        .squash_merge(&parent_wt, "child", "Squashed child work")
        .unwrap();

    let parent_after = rev_parse(&bare, "parent");
    assert_ne!(parent_after, parent_before);
    assert_eq!(parent_count(&bare, "parent"), 1);
    assert!(parent_wt.join("work.md").is_file());
    // The squash commit carries the message.
    let output = std::process::Command::new("git")
        .args(["-C", parent_wt.as_str(), "log", "-1", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Squashed child work"
    );
}

#[test]
fn fast_forward_to_advances_the_checked_out_branch() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    let child_tip = rev_parse(&bare, "child");
    let parent_before = rev_parse(&bare, "parent");

    System.fast_forward_to(&parent_wt, &child_tip).unwrap();

    let parent_after = rev_parse(&bare, "parent");
    assert_eq!(parent_after, child_tip);
    assert_ne!(parent_after, parent_before);
    assert!(parent_wt.join("work.md").is_file());
}

#[test]
fn fast_forward_to_refuses_a_diverged_target() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    // Parent diverges: its own commit lands while child sits elsewhere.
    std::fs::write(parent_wt.join("parent-only.md"), "mine\n").unwrap();
    git(&parent_wt, &["add", "parent-only.md"]);
    git(&parent_wt, &["commit", "-m", "parent drift"]);

    let child_tip = rev_parse(&bare, "child");
    let error = System
        .fast_forward_to(&parent_wt, &child_tip)
        .expect_err("diverged — cannot fast-forward");

    assert!(matches!(error, Error::Refused { .. }));
}

/// The local-integration contract: nothing about the parent changes while a
/// candidate is being built and checked, no matter which strategy the
/// candidate uses. Only an explicit `fast_forward_to` (rebase) or the actual
/// merge (merge/squash) moves it.
#[test]
fn the_parent_is_untouched_until_the_merge_is_explicitly_invoked() {
    let (guard, bare, parent_wt) = integration_repo();
    let _ = guard;
    let child_tip = rev_parse(&bare, "child");
    let parent_before = rev_parse(&bare, "parent");
    let parent_files: Vec<Utf8PathBuf> = std::fs::read_dir(&parent_wt)
        .unwrap()
        .map(|entry| Utf8PathBuf::from_path_buf(entry.unwrap().path()).unwrap())
        .collect();

    // A detached candidate per strategy: each stages the child's work, and
    // none of them touches the parent's branch or files. The rebase candidate
    // starts at the parent's tip and fast-forwards to the child's — the
    // "rebase then ff" topology — while merge/squash start at the child's tip
    // and fold the child's commits in.
    let merge: &dyn Fn(&Utf8Path) -> Result<(), Error> = &|wt| System.merge_no_ff(wt, "child");
    let squash: &dyn Fn(&Utf8Path) -> Result<(), Error> =
        &|wt| System.squash_merge(wt, "child", "squash");
    let rebase: &dyn Fn(&Utf8Path) -> Result<(), Error> =
        &|wt| System.fast_forward_to(wt, &child_tip);
    for (name, revision, stage) in [
        ("merge", &parent_before, merge),
        ("squash", &parent_before, squash),
        ("rebase", &parent_before, rebase),
    ] {
        // Candidates live beside the parent worktree, never inside it — a
        // nested worktree would pollute the parent's own file listing.
        let candidate = parent_wt
            .parent()
            .unwrap()
            .join(format!("candidate-{name}"));
        System
            .add_detached_worktree(&bare, &candidate, revision)
            .unwrap();
        stage(&candidate).unwrap();
        assert!(candidate.join("work.md").is_file());
    }

    // The parent is byte/ref identical throughout.
    assert_eq!(rev_parse(&bare, "parent"), parent_before);
    let parent_files_after: Vec<Utf8PathBuf> = std::fs::read_dir(&parent_wt)
        .unwrap()
        .map(|entry| Utf8PathBuf::from_path_buf(entry.unwrap().path()).unwrap())
        .collect();
    assert_eq!(parent_files_after, parent_files);
    assert!(
        !parent_wt.join("work.md").exists(),
        "the candidate's work must not leak into the parent worktree"
    );

    // The explicit merge moves the parent.
    System.merge_no_ff(&parent_wt, "child").unwrap();
    assert_ne!(rev_parse(&bare, "parent"), parent_before);
}
