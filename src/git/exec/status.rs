use camino::{Utf8Path, Utf8PathBuf};

use super::super::Error;
use super::{git, run};

/// `git -C <worktree> status --porcelain` — whether the worktree holds
/// uncommitted changes.
///
/// Porcelain output is empty exactly when the worktree is clean, so the
/// boolean is the non-emptiness of the captured stdout. Untracked files count
/// as dirty — a push does not carry them, and the preview saying "clean" while
/// `git status` disagrees would be a lie the human acts on.
pub(crate) fn worktree_dirty(path: &Utf8Path) -> Result<bool, Error> {
    let stdout = run(&git().cwd(path).arg("status").arg("--porcelain"))?;
    Ok(!stdout.is_empty())
}

/// `git -C <worktree> status --porcelain -z --untracked-files=all` — every
/// path in the worktree that diverges from its last commit, as
/// worktree-relative paths. Tracked edits and untracked files alike, which is
/// the difference that matters to the caller: a file created during a run is
/// untracked and invisible to `git diff`.
///
/// `-z` rather than the default line format because the default *quotes* any
/// path holding a space or a non-ASCII byte, and a quoted path is one the
/// caller would have to unquote correctly before deciding whether a write
/// contract covers it. Whether a write is refused must not hinge on getting
/// an escaping dialect right; NUL-separated records need no quoting at all.
///
/// `--untracked-files=all` rather than the default `normal`, which collapses
/// a new directory into the directory name alone — one entry standing for any
/// number of files, none of them named.
///
/// A rename emits two records, the new path then the original. Both are
/// returned: both are writes, since the file at the old path is gone.
pub(crate) fn changed_paths(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    let stdout = run(&git()
        .cwd(path)
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .arg("--untracked-files=all"))?;
    Ok(parse_status_z(&stdout))
}

/// The paths named by `git status --porcelain -z` output.
///
/// Each record is `XY<space><path>`: two status columns, then the path. A
/// record whose first column is `R` or `C` (rename, copy) is followed by a
/// second record carrying the origin path, which is taken as well.
fn parse_status_z(stdout: &str) -> Vec<Utf8PathBuf> {
    let mut paths = Vec::new();
    let mut records = stdout.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut columns = record.chars();
        let (Some(index_status), Some(_worktree_status)) = (columns.next(), columns.next()) else {
            continue;
        };
        let entry = columns.as_str().trim_start();
        if entry.is_empty() {
            continue;
        }
        paths.push(Utf8PathBuf::from(entry));
        if matches!(index_status, 'R' | 'C')
            && let Some(origin) = records.next()
        {
            paths.push(Utf8PathBuf::from(origin));
        }
    }
    paths
}

/// `git -C <worktree> rev-parse HEAD` — the commit the worktree's branch is
/// on right now.
///
/// The fixed point the write-contract audit takes its post-run diff against.
/// The audit's other oracle, [`changed_paths`], reports divergence from the
/// *current* commit, so a `git commit` empties it — the run's writes are
/// still on the branch, but nothing diverges from HEAD any more. Recording
/// where HEAD was before the child started is what makes the committed half
/// of a run visible at all; see [`paths_committed_since`].
pub(crate) fn head_commit(path: &Utf8Path) -> Result<String, Error> {
    let command = git().cwd(path).arg("rev-parse").arg("HEAD");
    let stdout = run(&command)?;
    let sha = stdout.trim();
    if sha.is_empty() {
        return Err(Error::Refused {
            command: format!("git -C {path} rev-parse HEAD"),
            detail: "expected a commit id, got nothing".to_owned(),
        });
    }
    Ok(sha.to_owned())
}

/// `git -C <worktree> diff --name-only -z <since> HEAD` — every path whose
/// content differs between the commit `since` and the worktree's current
/// commit.
///
/// A *tree* comparison, not a walk of the commits between them, which is the
/// property that matters: it answers the same way whether the run committed
/// once, committed ten times, amended, rebased, reset, or switched the
/// worktree onto another branch entirely. Any of those leave `since` and
/// `HEAD` as two commits with a diff, and none of them are reachable through
/// the reflog reasoning a commit walk would need.
///
/// `-z` for the same reason [`changed_paths`] uses it: the default format
/// quotes any path holding a space or a non-ASCII byte, and whether a write
/// contract covers a path must not hinge on unquoting it correctly.
pub(crate) fn paths_committed_since(
    path: &Utf8Path,
    since: &str,
) -> Result<Vec<Utf8PathBuf>, Error> {
    let stdout = run(&git()
        .cwd(path)
        .arg("diff")
        .arg("--name-only")
        .arg("-z")
        .arg(since)
        .arg("HEAD"))?;
    Ok(stdout
        .split('\0')
        .filter(|record| !record.is_empty())
        .map(Utf8PathBuf::from)
        .collect())
}

/// `git -C <worktree> diff HEAD` — the worktree's uncommitted divergence from
/// its last commit, staged and unstaged.
///
/// Empty when the worktree is clean. Untracked files are invisible to
/// `git diff` by design, so "clean" means "no tracked content diverged" — the
/// caller (reconcile) wants the code divergence an executor left uncommitted,
/// which is always a tracked edit.
pub(crate) fn diff_worktree(path: &Utf8Path) -> Result<String, Error> {
    run(&git().cwd(path).arg("diff").arg("HEAD"))
}

/// `git show <commit> --format= | git patch-id --stable` — the stable
/// patch-id of one commit's diff.
///
/// Patch-id fingerprints "the same change" across authorship, message, and
/// rebase, so it is how a commit re-landed upstream under a new identity is
/// recognised. The `--format=` drops the commit header and keeps only the
/// diff, which is what `patch-id` hashes.
pub(crate) fn commit_patch_id(worktree: &Utf8Path, commit: &str) -> Result<String, Error> {
    let diff = run(&git().cwd(worktree).arg("show").arg("--format=").arg(commit))?;
    patch_id_of(worktree, &diff)
}

/// `git diff <base> <tip> | git patch-id --stable` — the stable patch-id of
/// the cumulative diff between two revisions.
///
/// The squash-shaped counterpart to [`commit_patch_id`]: it fingerprints a
/// *range* of commits as one change, which is what a squash-merged re-landing
/// of several local commits looks like upstream.
pub(crate) fn diff_patch_id(worktree: &Utf8Path, base: &str, tip: &str) -> Result<String, Error> {
    let diff = run(&git().cwd(worktree).arg("diff").arg(base).arg(tip))?;
    patch_id_of(worktree, &diff)
}

/// `git patch-id --stable`, fed `diff` on stdin — the hash in the first
/// column of its output.
fn patch_id_of(worktree: &Utf8Path, diff: &str) -> Result<String, Error> {
    let stdout = run(&git().arg("patch-id").arg("--stable").stdin(diff))?;
    let id = stdout.split_whitespace().next().unwrap_or_default();
    if id.is_empty() {
        return Err(Error::Refused {
            command: format!("git -C {worktree} patch-id --stable"),
            detail: format!("expected a patch-id, got `{stdout}`"),
        });
    }
    Ok(id.to_owned())
}
