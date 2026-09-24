use camino::{Utf8Path, Utf8PathBuf};

use crate::infra::fs;

use super::super::{Error, WorktreeEntry};
use super::{git, run};

/// `git --git-dir <git_dir> worktree add <dest> <branch>`.
///
/// `branch` must already exist. Creating a branch as part of adding a worktree
/// (`worktree add -b`) is a feature-slice concern; `sync` only ever materialises
/// a branch the remote already has.
pub(crate) fn add_worktree(git_dir: &Utf8Path, dest: &Utf8Path, branch: &str) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("add")
        .arg(dest.as_str())
        .arg(branch))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree add -b <branch> <dest> <from_branch>`.
///
/// The one operation that creates a branch *and* a worktree in a single git
/// call — what `feature promote` needs, and the reason `sync`'s
/// [`add_worktree`] stays strictly branch-exists-only: sync materialises what
/// the remote already has, promote is where new branches are born.
pub(crate) fn create_branch_and_worktree(
    git_dir: &Utf8Path,
    branch: &str,
    from_branch: &str,
    dest: &Utf8Path,
) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch)
        .arg(dest.as_str())
        .arg(from_branch))?;
    Ok(())
}

/// `git -C <worktree> fetch --prune --quiet origin <branch>`.
///
/// Runs *inside* the worktree, not against the bare, so what it fetches lands
/// in `FETCH_HEAD` and no branch ref moves — the fast-forward is the separate,
/// deliberate next step, and a feature worktree sharing this bare's refs is
/// untouched by a default-branch refresh.
///
/// The tracking refspec still applies: git updates
/// `refs/remotes/origin/<branch>` opportunistically alongside `FETCH_HEAD`, so
/// a `--force-with-lease` from this worktree has something to lease against
/// after a `repo pull`.
pub(crate) fn fetch_branch(worktree: &Utf8Path, branch: &str) -> Result<(), Error> {
    let prefix = run(&git()
        .cwd(worktree)
        .arg("config")
        .arg("--get")
        .arg(super::clone::REF_PREFIX_KEY))
    .map(|s| s.trim().to_owned())
    .unwrap_or_default();

    run(&git()
        .cwd(worktree)
        .arg("fetch")
        .arg("--prune")
        .arg("--quiet")
        .arg("origin")
        .arg(format!("{prefix}{branch}")))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree remove --force <dest>`.
///
/// `--force` because a worktree with uncommitted changes is refused by git
/// otherwise, and this is only called from a cascade that has decided the
/// work is being torn down.
pub(crate) fn remove_worktree(git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(dest.as_str()))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree add --detach <dest> <revision>` — a
/// throwaway worktree on no branch, for staging a candidate integration
/// without moving any branch. The candidate can be built and checked there
/// while the parent's branch stays untouched.
pub(crate) fn add_detached_worktree(
    git_dir: &Utf8Path,
    dest: &Utf8Path,
    revision: &str,
) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(dest.as_str())
        .arg(revision))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree move <from> <to>` — relocate a
/// worktree's directory and repair the bare repository's registration of it.
///
/// `to`'s parent is created first when missing, since a branch-derived
/// destination may nest one directory deeper than any sibling worktree does.
pub(crate) fn move_worktree(
    git_dir: &Utf8Path,
    from: &Utf8Path,
    to: &Utf8Path,
) -> Result<(), Error> {
    if let Some(parent) = to.parent() {
        fs::ensure_dir(parent)?;
    }
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("move")
        .arg(from.as_str())
        .arg(to.as_str()))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree list --porcelain`, parsed.
pub(crate) fn list_worktrees(git_dir: &Utf8Path) -> Result<Vec<WorktreeEntry>, Error> {
    let out = run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("list")
        .arg("--porcelain"))?;
    Ok(parse_worktree_list(&out))
}

/// `git --git-dir <git_dir> worktree prune` — drop registrations whose
/// directory no longer exists.
pub(crate) fn prune_worktrees(git_dir: &Utf8Path) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("prune"))?;
    Ok(())
}

/// Parse `git worktree list --porcelain` records; bare and detached entries
/// carry no branch.
pub(crate) fn parse_worktree_list(porcelain: &str) -> Vec<WorktreeEntry> {
    porcelain
        .split("\n\n")
        .filter_map(|record| {
            let mut lines = record.lines();
            let path = lines.next()?.strip_prefix("worktree ")?;
            let mut entry = WorktreeEntry {
                path: Utf8PathBuf::from(path),
                branch: None,
                detached: false,
                prunable: false,
            };
            for line in lines {
                if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                    entry.branch = Some(branch.to_owned());
                } else if line == "detached" {
                    entry.detached = true;
                } else if line == "prunable" || line.starts_with("prunable ") {
                    entry.prunable = true;
                }
            }
            Some(entry)
        })
        .collect()
}
