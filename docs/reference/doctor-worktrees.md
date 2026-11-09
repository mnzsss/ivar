# `ivar doctor`: orphan worktrees

Part of the [command reference](commands.md). A worktree of a repo on a branch no feature
owns is reported as `repo.worktree_orphaned`. The finding says whether it can go, and lists
the facts behind that verdict, so none of them needs checking by hand.

## The facts

| Fact | Where it comes from |
| --- | --- |
| uncommitted changes | `git status`: tracked and untracked files, a count of paths |
| commits not pushed | commits ahead of `origin/<branch>`, the remote-tracking ref |
| remote branch | whether `origin/<branch>` exists: "deleted or never pushed" when it does not |
| integrated into the default branch | local comparison, below |
| pull request | `gh`, only with `--github`: merged, closed without merging, open, or none |

The remote-tracking refs are as of the last fetch: `ivar doctor` makes no network call to read
them, and `ivar repo pull` refreshes the default branch's. A stale `origin/<default>` that predates
a squash-merge makes the worktree look unintegrated, which errs toward keeping it; `git fetch
--prune` in the repo's `.bare` brings every ref up to date.

The local comparison uses `origin/<default>` when it exists, else the local default branch. The
worktree is integrated when it has no commits the default branch lacks, or when it has some but
every file changed by `git diff <default>...HEAD` is identical, mode and content, on the default
branch. The second case is what a squash-merge leaves: the branch's commits are not in the
default branch's history, but its files are.

## The verdict

**Safe to remove (integrated, clean)** needs all of these:

- no uncommitted changes;
- no commits ahead of `origin/<branch>` (a branch with no remote counterpart has none to be
  ahead of, so this holds trivially);
- the work is integrated: a pull request merged at exactly the worktree's `HEAD`, or the local
  comparison above.

Anything else is **needs a look (unsent or unmerged work)**. A fact that cannot be read, such as
a `git status` that fails, is never read as good news. A file the default branch changed again
after the merge differs from the branch's copy, so that worktree needs a look: the comparison errs
toward keeping work.

## `--github`

`ivar doctor --github` asks `gh pr list --head <branch> --state all` in each orphan's repo. It is
a network call, so it runs only with the flag. Without `gh`, when not signed in, or offline, the
finding says the pull request is unknown and the local comparison decides alone.

A pull request closed without merging, still open, or merged at a different commit than the
worktree's `HEAD` is shown but never counts as integration: the work it carried may not have
landed.

## `--fix`

`ivar doctor --fix` removes the orphan worktrees classified safe, and no others. It lifts the
read-only guard of a checkout that has one, as the removal command in the finding does. It
deletes no branch, and it makes no network call. Removed paths are listed in the output, and in
`removed` with `--json`; a removal that fails leaves the finding, with the reason.

## The owner of the folder

When the folder under `repos/<repo>/` is named for a branch other than the one checked out in it,
the finding names the feature whose branch the folder is named for, if its record is still
there.
