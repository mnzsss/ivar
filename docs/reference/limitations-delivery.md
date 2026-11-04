# Limitations of delivery and the default branch

What `ivar` does when it pushes, and what protects the default branch. See [Limitations and failure modes](limitations.md) for the rest of `ivar`.

## No atomic push across N remotes

`deliver --land` merges every promoted repo locally or none of them — that part
is atomic, because it is local. The pushes that follow are independent network
operations against independent remotes, and no protocol makes them one
transaction.

"Atomic locally" is enforced by compensation, not by a transaction: the repos
have separate Git directories, so there is nothing to commit or abort across
them. Land records each default's original commit before merging, and if any
repo's fast-forward fails, it resets the ones already merged back to those
commits. That compensation can itself fail — a repo it could not restore is
named in `deliver.land_rollback_failed`, alongside the merge failure that
triggered the rollback. When that happens the batch is genuinely half-landed,
and the failure says so rather than reporting all-or-nothing it did not
achieve.

The honest failure mode: every repo's default carries the change locally, and
one of them did not reach its remote. `deliver.land_push_failed` names the repo;
the local default is ahead of its remote until pushed. Rerunning the land is
safe — a repo already merged is already a fast-forward no-op.

`ivar` does not roll back merges that succeeded to match a push that failed.
Reverting shared history to compensate for a network error is worse than an
unpushed commit.

## What protects the default branch

Protection is layered, and the layers are not equally strong. Each one below
states what it stops and what it does not.

**The structured-tool guard** denies Write, Edit, MultiEdit, NotebookEdit, and
the patch tools when their target is outside the session's writable set. Tool
names are matched after normalising case and separators, so `notebook_edit` and
`NotebookEdit` are the same tool. This layer acts as the advisory diagnosis
mechanism for tool calls, providing immediate actionable feedback to the agent.

**A Discovery Session resolves to a set holding the view dir alone.** A session
with no promoted feature may write its own notes and nothing else. This closes a
gap where a session with no feature previously resolved to no set at all, which
disarmed the guard entirely.

**Shell commands are governed at the kernel boundary on Linux.** At the hook layer,
shell commands are not inspected: pattern-matching arbitrary shell syntax is unreliable.
Instead, on Linux, Landlock restricts the spawned shell and all child processes
at the syscall layer. On platforms without Landlock, shell writes outside promoted
worktrees remain unblocked by the advisory hook layer.
**The pre-commit hook is deterministic and does not depend on the provider.**
`ivar sync` installs a `pre-commit` hook that refuses any commit on a repo's
default branch, however that commit is invoked — through a tool call, through
Bash, or by hand. It lives in `<bare>/ivar-hooks/` and is selected per worktree
via worktree-local `core.hooksPath`, so a project's own hook manager (husky and
similar, which write `core.hooksPath` into the shared config during
`pnpm install`) does not displace it, and feature worktrees keep the project's
hooks. Because it is worktree-local, it binds the default-branch worktree only.

`ivar feature deliver` commits onto the default branch by design when landing with the
squash strategy, and passes `--no-verify` at that one call site. This is not a
general escape hatch: there is no environment variable and no configuration that
turns the hook off.

**The read-only guard is applied to the worktree root only.** It is not applied
recursively, because worktrees are full of files hardlinked out of a package
manager's content-addressed store, and `chmod` acts on the inode: recursing
would change permissions inside that store and inside every checkout sharing it.
A root-only guard stops files being created or removed in the worktree; it does
not stop an existing tracked file from being modified in place.

### What is not covered

- **File metadata operations.** Landlock handles file content and directory structure
  modifications (`open` with write flags, `mkdir`, `unlink`, `rename`, `rmdir`). It
  does not restrict metadata operations such as `stat`, `chmod`, `chown`, `utime`,
  or `flock`.
- **Platforms without Landlock.** On macOS and older Linux kernels (< 5.13), kernel
  sandboxing is unavailable; protection degrades gracefully to the advisory tool guard.
- **T3 Code sessions.** A session opened with `ivar session start --provider t3` has no kernel sandbox on any platform, including Linux with Landlock; the notes on shell writes and structured tools in "Session write guard: Enforcing (Linux) vs Advisory (Everywhere)" and "What protects the default branch" apply to it as they do to a platform without Landlock.
- **Repositories outside the hall.** Protection is installed by `ivar sync` on
  repos declared in `ivar.json`. A clone made by hand elsewhere has none of it.
- **`git commit --no-verify`,** typed by a human or emitted by an agent. Git
  offers no way for a hook to refuse this.
None of this is a security boundary. It is a guardrail against plausible
mistakes — an agent committing to `main` because that is where it happened to
be — and it is built for a hall where the person running it wants the guardrail.
It will not stop a determined process running as your user.
