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

use crate::infra::{fs, hash};

use super::{BlobEvidence, CommitInfo, Divergence, Error, TargetState};

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

/// Every local branch in the repository at `git_dir`, without the
/// `refs/heads/` prefix, sorted lexically.
///
/// Reads the real refs, not `HEAD`, so a repository whose default branch has
/// no commits yet still answers — with an empty list, since an unborn branch
/// has no ref for `list_branches` to find. Sorted because git2 iterates in
/// an unspecified order and every caller renders a list.
pub(crate) fn list_branches(git_dir: &Utf8Path) -> Result<Vec<String>, Error> {
    let repository = open(git_dir)?;

    let mut branches = Vec::new();
    for branch in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|source| Error::NotARepository {
            path: git_dir.to_path_buf(),
            detail: source.message().to_owned(),
        })?
    {
        let (branch, _) = branch.map_err(|source| Error::NotARepository {
            path: git_dir.to_path_buf(),
            detail: source.message().to_owned(),
        })?;
        if let Some(name) = branch.name().map_err(|_| Error::NotUtf8 {
            display: "non-UTF-8 branch name".to_owned(),
        })? {
            branches.push(name.to_owned());
        }
    }
    branches.sort();
    Ok(branches)
}

/// Whether `ancestor` is an ancestor of `descendant` in the repository at
/// `git_dir` — `git merge-base --is-ancestor <ancestor> <descendant>`, through
/// `git2::Repository::graph_descendant_of`.
///
/// Both revisions must exist; a missing one is git's own refusal, surfaced as
/// [`Error::Refused`] with git's own sentence — never `Ok(false)`, which would
/// read as "not an ancestor" and hide that the revision was never there.
pub(crate) fn is_ancestor(
    git_dir: &Utf8Path,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, Error> {
    let repository = open(git_dir)?;

    let ancestor_id = resolve(&repository, git_dir, ancestor)?;
    let descendant_id = resolve(&repository, git_dir, descendant)?;

    // `graph_descendant_of` does not consider a commit its own descendant,
    // but `git merge-base --is-ancestor` — the command this function's own
    // doc says it is — does: a commit is an ancestor of itself, exit 0. A
    // branch sitting exactly at its base's tip, with no commits of its own
    // (or freshly `rebase --onto`d there), hits this path, and reporting it
    // as "not an ancestor" would be a false "base moved" for a legitimate
    // state.
    if ancestor_id == descendant_id {
        return Ok(true);
    }

    repository
        .graph_descendant_of(descendant_id, ancestor_id)
        .map_err(|source| Error::Refused {
            command: format!("git -C {git_dir} merge-base --is-ancestor {ancestor} {descendant}"),
            detail: source.message().to_owned(),
        })
}

/// Resolve `rev` to a commit id in `repository`, or say why it could not be —
/// git's own sentence for a revision that does not exist.
fn resolve(
    repository: &git2::Repository,
    git_dir: &Utf8Path,
    rev: &str,
) -> Result<git2::Oid, Error> {
    repository
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map(|commit| commit.id())
        .map_err(|source| Error::Refused {
            command: format!("git -C {git_dir} rev-parse {rev}"),
            detail: source.message().to_owned(),
        })
}

/// The commit id `revision` names in the repository at `git_dir` —
/// `git rev-parse <revision>`, through git2.
///
/// A revision that does not exist is [`Error::Refused`] with git's own
/// sentence, never `Ok` — receipt freshness reads the child branch's tip and
/// must distinguish "branch moved" from "branch never existed".
pub(crate) fn revision_commit(git_dir: &Utf8Path, revision: &str) -> Result<String, Error> {
    let repository = open(git_dir)?;
    let id = resolve(&repository, git_dir, revision)?;
    Ok(id.to_string())
}

/// The commit both `a` and `b` descend from — `git merge-base <a> <b>`,
/// through git2.
///
/// Either revision must exist; a missing one is [`Error::Refused`] with git's
/// own sentence.
pub(crate) fn merge_base(git_dir: &Utf8Path, a: &str, b: &str) -> Result<String, Error> {
    let repository = open(git_dir)?;
    let a_id = resolve(&repository, git_dir, a)?;
    let b_id = resolve(&repository, git_dir, b)?;

    let base = repository
        .merge_base(a_id, b_id)
        .map_err(|source| Error::Refused {
            command: format!("git -C {git_dir} merge-base {a} {b}"),
            detail: source.message().to_owned(),
        })?;
    Ok(base.to_string())
}

/// How `local` and `remote` diverge in the repository at `git_dir`.
///
/// `local_only` is the commits reachable from `local` and not from `remote`;
/// `remote_only` the mirror image. Both lists are newest-first, in git's own
/// walk order.
pub(crate) fn divergence(
    git_dir: &Utf8Path,
    local: &str,
    remote: &str,
) -> Result<Divergence, Error> {
    let repository = open(git_dir)?;
    let local_id = resolve(&repository, git_dir, local)?;
    let remote_id = resolve(&repository, git_dir, remote)?;

    let local_only = commits_in(&repository, git_dir, local_id, remote_id)?;
    let remote_only = commits_in(&repository, git_dir, remote_id, local_id)?;

    Ok(Divergence {
        local_only,
        remote_only,
    })
}

/// Every commit reachable from `tip` and not from `base`, newest-first.
fn commits_in(
    repository: &git2::Repository,
    git_dir: &Utf8Path,
    tip: git2::Oid,
    base: git2::Oid,
) -> Result<Vec<CommitInfo>, Error> {
    let mut walk = repository
        .revwalk()
        .map_err(|source| Error::NotARepository {
            path: git_dir.to_path_buf(),
            detail: source.message().to_owned(),
        })?;
    walk.push(tip).map_err(|source| Error::Refused {
        command: format!("git -C {git_dir} rev-list --oneline {tip}.."),
        detail: source.message().to_owned(),
    })?;
    walk.hide(base).map_err(|source| Error::Refused {
        command: format!("git -C {git_dir} rev-list --oneline ..{base}"),
        detail: source.message().to_owned(),
    })?;

    let mut commits = Vec::new();
    for id in walk {
        let id = id.map_err(|source| Error::Refused {
            command: format!("git -C {git_dir} rev-list --oneline {tip}..{base}"),
            detail: source.message().to_owned(),
        })?;
        let commit = repository
            .find_commit(id)
            .map_err(|source| Error::NotARepository {
                path: git_dir.to_path_buf(),
                detail: source.message().to_owned(),
            })?;
        let subject = commit
            .summary()
            .map_err(|source| Error::Refused {
                command: format!("git -C {git_dir} log --format=%s {id}"),
                detail: source.message().to_owned(),
            })?
            .unwrap_or("")
            .to_owned();
        commits.push(CommitInfo {
            sha: id.to_string(),
            subject,
        });
    }
    Ok(commits)
}

/// What `path` held in `commit`, in the repository at `worktree`.
///
/// Reads the commit's *tree*, not the index and not the worktree, which is
/// what makes the answer stable across everything a run can do to HEAD:
/// commit, amend, reset, rebase, or switch branches. The starting commit still
/// exists, and it still holds what it held.
///
/// `Ok(None)` for a path the tree has nothing at, and for one that names
/// anything other than a blob — a directory or a submodule gitlink. Neither is
/// a file a receipt describes, and neither is reachable from the path sets
/// that feed this (`git status` and `git diff --name-only` name blobs).
///
/// The hash is SHA-256 of the blob's bytes, not git's own object id, so it
/// compares equal to a hash taken over the same path in the worktree. For a
/// symlink the blob's bytes are the link target — the same convention
/// `PathEvidence::symlink` records on the domain side.
pub(crate) fn path_at_commit(
    worktree: &Utf8Path,
    commit: &str,
    path: &Utf8Path,
) -> Result<Option<BlobEvidence>, Error> {
    let repository = open(worktree)?;
    let id = resolve(&repository, worktree, commit)?;

    let tree = repository
        .find_commit(id)
        .and_then(|commit| commit.tree())
        .map_err(|source| Error::Refused {
            command: format!("git -C {worktree} cat-file -p {commit}^{{tree}}"),
            detail: source.message().to_owned(),
        })?;

    let entry = match tree.get_path(path.as_std_path()) {
        Ok(entry) => entry,
        // The one error that is an answer rather than a failure: git reports a
        // path the tree does not hold as `NotFound`, and "nothing was there"
        // is exactly the baseline an added file needs.
        Err(source) if source.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Refused {
                command: format!("git -C {worktree} cat-file -p {commit}:{path}"),
                detail: source.message().to_owned(),
            });
        }
    };

    if entry.kind() != Some(git2::ObjectType::Blob) {
        return Ok(None);
    }

    let blob = repository
        .find_blob(entry.id())
        .map_err(|source| Error::Refused {
            command: format!("git -C {worktree} cat-file blob {}", entry.id()),
            detail: source.message().to_owned(),
        })?;

    Ok(Some(BlobEvidence {
        // `filemode` is `i32` in git2 and every real value (`100644`,
        // `100755`, `120000`) fits a `u32`. A negative one would be a libgit2
        // bug, not a repository state, so it reads as 0 rather than panicking
        // in the middle of an audit.
        mode: u32::try_from(entry.filemode()).unwrap_or(0),
        sha256: hash::bytes(blob.content()),
    }))
}

/// Open `path` as a repository, or say clearly that it is not one.
fn open(path: &Utf8Path) -> Result<git2::Repository, Error> {
    git2::Repository::open(path).map_err(|source| Error::NotARepository {
        path: path.to_path_buf(),
        detail: source.message().to_owned(),
    })
}

#[cfg(test)]
#[path = "../../tests/unit/git/read.rs"]
mod tests;
