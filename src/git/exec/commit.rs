use camino::{Utf8Path, Utf8PathBuf};

use super::super::{CommitOutcome, Error, read};
use super::{git, run};

/// Pathspec magic that leaves every `.env` file out of a sweep, wherever it is.
///
/// A hall's `.gitignore` is what normally keeps a `.env` out of git, and a
/// hall whose `.gitignore` lost that line would otherwise have its secrets
/// swept into a commit and pushed by a command nobody is watching. Deliberately
/// not configurable: no automated commit wants a `.env`.
const NEVER_SWEPT: &str = ":(exclude,glob)**/.env";

/// Whether `path` names a file no automated commit may take.
fn never_swept(path: &Utf8Path) -> bool {
    path.file_name() == Some(".env")
}

/// `git commit --only -m <message> -- <paths>` — one commit holding the
/// changes to `paths` and nothing else the worktree has pending.
///
/// `git add <paths> && git commit` would be the obvious spelling and the wrong
/// one: the commit takes the whole index, so whatever the user had staged for
/// other paths would ride along under a message that does not describe it.
/// `--only` commits the named paths from the worktree and leaves the index
/// alone for everything else.
///
/// Which paths are *pending* is asked of `git status` with the paths as
/// pathspecs, and that one question settles three things at once: a path
/// `.gitignore` ignores is not listed (an explicit pathspec does not override
/// it), a path git has never seen and that no longer exists is not listed (and
/// `--only` would fail on it), and a directory is expanded to the files in it.
///
/// `--only` also refuses an untracked path, so those get an intent-to-add entry
/// first (`git add -N`), which records no content and stages nothing. If the
/// commit is refused, those entries are removed again so a refusal leaves the
/// index as it found it.
pub(crate) fn commit_paths(
    worktree: &Utf8Path,
    paths: &[Utf8PathBuf],
    message: &str,
) -> Result<CommitOutcome, Error> {
    if let Some(reason) = read::repo_state(worktree)?.skip_reason() {
        return Ok(CommitOutcome::Skipped(reason));
    }
    let inside: Vec<&Utf8PathBuf> = paths.iter().filter(|path| stays_inside(path)).collect();
    if inside.is_empty() {
        return Ok(CommitOutcome::Nothing);
    }
    let pending = pending(worktree, &inside)?;
    if pending.is_empty() {
        return Ok(CommitOutcome::Nothing);
    }
    let untracked: Vec<&Utf8PathBuf> = pending
        .iter()
        .filter(|entry| entry.untracked)
        .map(|entry| &entry.path)
        .collect();
    if !untracked.is_empty() {
        run(&literal(worktree)
            .arg("add")
            .arg("--intent-to-add")
            .arg("--")
            .args(untracked.iter().map(|path| path.as_str())))?;
    }
    let committed = run(&literal(worktree)
        .arg("commit")
        .arg("--only")
        .arg("-m")
        .arg(message)
        .arg("--")
        .args(pending.iter().map(|entry| entry.path.as_str())));
    if committed.is_err() && !untracked.is_empty() {
        let _ = run(&literal(worktree)
            .arg("reset")
            .arg("--quiet")
            .arg("--")
            .args(untracked.iter().map(|path| path.as_str())));
    }
    committed?;
    Ok(CommitOutcome::Committed)
}

/// `git add -A` then `git commit -m <message>` — everything pending, staged or
/// not, new, modified or deleted, minus what `.gitignore` ignores and any
/// `.env` file ([`NEVER_SWEPT`]).
///
/// "Nothing pending" is decided after the add, from the index (`git diff
/// --cached --quiet`), so a change the user already staged counts and a tree
/// that only differs by ignored files does not.
pub(crate) fn commit_all(worktree: &Utf8Path, message: &str) -> Result<CommitOutcome, Error> {
    if let Some(reason) = read::repo_state(worktree)?.skip_reason() {
        return Ok(CommitOutcome::Skipped(reason));
    }
    run(&git()
        .cwd(worktree)
        .arg("add")
        .arg("-A")
        .arg("--")
        .arg(".")
        .arg(NEVER_SWEPT))?;
    if !staged_changes(worktree)? {
        return Ok(CommitOutcome::Nothing);
    }
    run(&git().cwd(worktree).arg("commit").arg("-m").arg(message))?;
    Ok(CommitOutcome::Committed)
}

/// `git diff --cached --quiet` — whether the index differs from `HEAD`.
///
/// Exit `1` is the answer "yes", not a failure; anything else non-zero is.
pub(crate) fn staged_changes(worktree: &Utf8Path) -> Result<bool, Error> {
    let command = git()
        .cwd(worktree)
        .arg("diff")
        .arg("--cached")
        .arg("--quiet");
    let output = crate::infra::proc::capture(&command)?;
    match output.code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(Error::Refused {
            command: command.display(),
            detail: output.diagnostic(),
        }),
    }
}

/// `git status --porcelain --untracked-files=no` — whether a tracked file
/// differs from the index, or the index from `HEAD`.
///
/// Untracked files are left out because they do not stop a rebase; tracked
/// changes do.
pub(crate) fn tracked_changes(worktree: &Utf8Path) -> Result<bool, Error> {
    let stdout = run(&git()
        .cwd(worktree)
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=no"))?;
    Ok(!stdout.is_empty())
}

/// `git add -- <paths>`.
pub(crate) fn stage_paths(worktree: &Utf8Path, paths: &[Utf8PathBuf]) -> Result<(), Error> {
    if paths.is_empty() {
        return Ok(());
    }
    run(&literal(worktree)
        .arg("add")
        .arg("--")
        .args(paths.iter().map(|path| path.as_str())))?;
    Ok(())
}

/// A `git` invocation in `worktree` that reads every path as a literal one.
///
/// A path named by a caller is a file, not a pattern: without this a file
/// called `[draft].md` would be a glob and `:(top)x` would be magic.
fn literal(worktree: &Utf8Path) -> crate::infra::proc::Command {
    git().cwd(worktree).arg("--literal-pathspecs")
}

/// One path with something pending.
struct Pending {
    path: Utf8PathBuf,
    /// Not known to git yet (`??`).
    untracked: bool,
}

/// The paths among `paths` that have something to commit, from
/// `git status --porcelain=v1 -z --untracked-files=all --no-renames`.
///
/// `-z` so that no path is quoted; `--no-renames` so that a move is a
/// deletion plus an addition, each of which `--only` can take as a path.
fn pending(worktree: &Utf8Path, paths: &[&Utf8PathBuf]) -> Result<Vec<Pending>, Error> {
    let stdout = run(&literal(worktree)
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-z")
        .arg("--untracked-files=all")
        .arg("--no-renames")
        .arg("--")
        .args(paths.iter().map(|path| path.as_str())))?;
    let mut found: Vec<Pending> = Vec::new();
    for record in stdout.split('\0') {
        let (Some(columns), Some(path)) = (record.get(..2), record.get(3..)) else {
            continue;
        };
        if path.is_empty()
            || never_swept(Utf8Path::new(path))
            || found.iter().any(|entry| entry.path == path)
        {
            continue;
        }
        found.push(Pending {
            path: Utf8PathBuf::from(path),
            untracked: columns == "??",
        });
    }
    Ok(found)
}

/// Whether `path` names something inside the worktree: relative, and never
/// stepping out of it. The commit helpers' own guarantee that no path outside
/// the worktree ever reaches a commit, whatever the caller passed.
fn stays_inside(path: &Utf8Path) -> bool {
    path.is_relative()
        && !path.as_str().is_empty()
        && path
            .components()
            .all(|part| !matches!(part, camino::Utf8Component::ParentDir))
}
