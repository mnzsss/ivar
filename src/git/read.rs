//! Reads, through `git2`. Never the network.
//!
//! ADR-0001 §3 splits git in two: this half answers questions, and it does so
//! in-process because spawning a subprocess to ask "is this a repository?"
//! costs more than the answer is worth, and because parsing porcelain output is
//! a second place for the answer to be wrong.
//!
//! `git2` is built with `default-features = false`, which leaves libgit2 with
//! no HTTPS and no SSH transport at all. That is not a limitation to work
//! around — it is the rule made structural. Anything here that tried to reach a
//! remote would fail to link, so the split cannot quietly erode.

use camino::{Utf8Path, Utf8PathBuf};

use crate::infra::fs;

use super::{Error, TargetState};

/// What is at `path`: a repository, something git does not recognise, or
/// nothing.
///
/// Exact, never a walk-up. `git2::Repository::open` does not search parent
/// directories (that is `discover`), and this module depends on that: `sync`
/// asks "is this worktree materialised?" about a specific directory, and a
/// walk-up answer would say yes for any empty directory inside a hall that
/// happens to be a git repository itself.
///
/// The `exists` check only runs when the path is *not* a repository, so the
/// steady-state answer costs one `Repository::open` and no extra `stat`.
pub(crate) fn target_state(path: &Utf8Path) -> Result<TargetState, Error> {
    if git2::Repository::open(path).is_ok() {
        return Ok(TargetState::Repository);
    }
    if fs::exists(path)? {
        return Ok(TargetState::Occupied);
    }
    Ok(TargetState::Absent)
}

/// The branch `HEAD` names in the repository at `git_dir`, without the
/// `refs/heads/` prefix.
///
/// Reads `HEAD` as a *symbolic* ref rather than resolving it to a commit, so a
/// repository whose default branch has no commits yet still answers. That is
/// not a corner case — `git clone --bare` of a repository created minutes ago
/// is exactly how a hall gets its first repo, and resolving would fail there
/// with "reference not found", which names the wrong problem.
pub(crate) fn head_branch(git_dir: &Utf8Path) -> Result<String, Error> {
    let repository = open(git_dir)?;

    let head = repository
        .find_reference("HEAD")
        .map_err(|source| Error::NotARepository {
            path: git_dir.to_path_buf(),
            detail: source.message().to_owned(),
        })?;

    // `symbolic_target` is fallible in git2 0.21 *and* optional, and the two
    // mean different things: `Err` is a ref name that is not valid UTF-8,
    // `Ok(None)` is a HEAD that points at a commit rather than a branch.
    // Collapsing them would report a detached HEAD for a repository that simply
    // has a branch named in some other encoding.
    let target = head
        .symbolic_target()
        .map_err(|source| Error::NotUtf8 {
            display: source.message().to_owned(),
        })?
        .ok_or_else(|| Error::DetachedHead {
            path: git_dir.to_path_buf(),
        })?;

    Ok(target
        .strip_prefix("refs/heads/")
        .unwrap_or(target)
        .to_owned())
}

/// The git administrative directory backing the worktree at `path`.
///
/// For a linked worktree this is `<bare>/worktrees/<name>/`, not the bare
/// repository. Bookkeeping written there has exactly the lifetime of the
/// worktree: `git worktree remove` takes it with it, so a rebuilt worktree
/// starts clean rather than inheriting a receipt describing a directory that no
/// longer exists.
pub(crate) fn worktree_git_dir(path: &Utf8Path) -> Result<Utf8PathBuf, Error> {
    let repository = open(path)?;
    let raw = repository.path().to_path_buf();

    Utf8PathBuf::from_path_buf(raw).map_err(|rejected| Error::NotUtf8 {
        display: rejected.to_string_lossy().into_owned(),
    })
}

/// Open `path` as a repository, or say clearly that it is not one.
fn open(path: &Utf8Path) -> Result<git2::Repository, Error> {
    git2::Repository::open(path).map_err(|source| Error::NotARepository {
        path: path.to_path_buf(),
        detail: source.message().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::git::exec;
    use crate::test_support::{empty_repo, seeded_repo, utf8_temp_dir};

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
}
