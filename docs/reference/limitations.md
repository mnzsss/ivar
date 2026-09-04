# Limitations and failure modes

What `ivar` does not do, and what breaks when you push on it. This page exists so
you find these here rather than by hitting them.

`ivar` isolates two things: **the filesystem** and **ports**. Everything below is
something outside those two.

## Shared daemon state — the big one

A `git worktree` gives every feature its own files. It does not give them their own
Postgres.

Two features promoting the same repo get two worktrees, two dev servers, two
distinct ports — and **one shared database**. Run a migration in one and the other
one sees it. Truncate a table in one and the other one's tests fail.

`ivar` does not solve this, and the honest reason is that the only general
solution is a container per environment, which `ivar` rejects by design: the whole
mechanism is real directories on your real filesystem that your real editor and
your real `lazygit` can open.

**What `ivar` gives you instead is the hook.** Each repo can carry a **session
hook** at `.ivar/setups/<repo>.session.sh`, run on every `ivar session start`,
in that repo's worktree, with the session identifier in its environment. The
person who knows how to isolate that repo is the person who wrote it — so `ivar`
hands them the seam rather than guessing.

Note which file this is. The setup script next to it
(`.ivar/setups/<repo>.sh`) is receipt-gated and runs about once per worktree —
right for `pnpm install`, useless for a database that must come up every session.
The hook is ungated and runs every time. A hook that fails warns and the session
still opens.

Two recipes that work:

```sh
# docker compose: one project name per session, so volumes do not collide
export COMPOSE_PROJECT_NAME="${IVAR_SESSION_ID}"
docker compose up -d

# postgres: one database per session
createdb "app_${IVAR_SESSION_ID}"
export DATABASE_URL="postgres:///app_${IVAR_SESSION_ID}"
```

That is not a workaround. It is the boundary: `ivar` owns worktrees and ports;
the repo owns its own state.

## Ports are reserved, not proxied

A session gets a range of ports reserved in `state.json`, and `ivar` writes them
into the session environment. That stops another `ivar` session from being handed
the same port. It does not make a process bind correctly, and it does not proxy
traffic.

A server that ignores the assigned port can still collide with another process.
A process that dies leaves its reserved port unavailable until the session is
stopped or cleaned up. `ivar status` reports the reservation; it cannot inspect
arbitrary processes to prove the port is free.

## Writable worktrees are shared within a feature

Promoting a repo makes its feature worktree writable. Every session on that
feature points at that same worktree. This is intentional: sessions cooperate on
one branch, rather than silently diverging into per-agent branches.

It means `ivar` does not prevent two agents from editing the same file. Git
serialises individual operations; it does not resolve concurrent intent. Use
coordination, reviews, or isolated branches when you need stronger separation.

## Execution is not a sandbox or a provider controller

Ivar does not sandbox provider activity. A provider can run shell commands,
formatters, and generators against any promoted worktree available to its
session. The Run Receipt records exact baseline and final snapshot evidence for
audit, but it neither attributes individual writes nor reverts them.

The active provider, not Ivar, creates, schedules, monitors, and synthesizes
native subagents. Ivar does not launch headless provider children, parse provider
transcripts, retain conversation or native-subagent identifiers, or promise that
a logical resume restores provider context. Resuming a receipt with another
provider attaches the current Feature Session to the same local audit record.

Use repository permissions, isolated environments, or a platform sandbox when
you need stronger write isolation.

## Local state is disposable

Folders like `skills/`, `skills-local/`, and `setups/` are managed or personal;
everything else under `.ivar/` is local derived state.
Removing it costs a re-clone and session recreation, not committed work. Do not
put source files, plans, secrets you need to share, or irreplaceable artifacts
there.

## A feature can span only registered repos

`ivar` cannot promote a repository it does not know about. Register it first with
`ivar repo add`; a path that merely happens to contain a Git checkout is not part
of the hall.

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

## Session write guard is best-effort

The session write guard (`ivar guard`) blocks structured writes (Write, Edit) that
target paths outside the session's view dir or promoted worktrees. It is a hook —
the provider invokes it before each tool call — and its effectiveness depends on
the provider respecting the hook protocol:

- **Claude Code:** The guard always exits 0; the deny decision travels in the JSON
  body. If the provider ignores the `permissionDecision: deny` response, the write
  proceeds.
- **OpenCode:** The guard exits non-zero for deny. A non-zero exit should abort the
  tool call, but this is provider behaviour, not ivar's guarantee.
- **OMP:** The guard exits 0 and outputs `{ "block": true, "reason": "..." }`. If
  the harness ignores the hook output, the write proceeds.
**What this means in practice:** the guard is an error message, not a firewall. A
provider that does not call the hook, or that ignores a deny decision, will allow
the write.


## Provider-specific capabilities and limitations

- **Session resume:** Claude Code supports resuming prior sessions (`--resume`).
  OpenCode and OMP do not support session resume; `ivar session start --resume`
  will reject attempts to resume with these providers.
- **MCP authentication scope:** OMP credentials installed via `ivar mcp auth` are
  scoped to the active omp profile (`OMP_PROFILE` or `PI_PROFILE`, defaulting to
  `default`). Switching profiles requires re-authenticating for that profile.
  However, credentials do not require manual re-authentication on expiry: omp
  natively refreshes tokens using the rendered `auth` block in `mcp.json`.
## What protects the default branch

Protection is layered, and the layers are not equally strong. Each one below
states what it stops and what it does not.

**The structured-tool guard** denies Write, Edit, MultiEdit, NotebookEdit, and
the patch tools when their target is outside the session's writable set. Tool
names are matched after normalising case and separators, so `notebook_edit` and
`NotebookEdit` are the same tool. This is the layer described above, and it
inherits that layer's weakness: it depends on the provider honouring the hook.

**A Discovery Session resolves to a set holding the view dir alone.** A session
with no promoted feature may write its own notes and nothing else. This closes a
gap where a session with no feature previously resolved to no set at all, which
disarmed the guard entirely.

**Shell commands are not classified.** `Bash` and every other shell tool are
allowed through without inspection. Deciding whether an arbitrary shell command
writes, and where, is not something a pattern match can do correctly, and a
classifier that is wrong in the permissive direction is worse than none: it
reads as protection while providing none. This is a deliberate gap, and it is
the largest one.

**The pre-commit hook is deterministic and does not depend on the provider.**
`ivar sync` installs a `pre-commit` hook that refuses any commit on a repo's
default branch, however that commit is invoked — through a tool call, through
Bash, or by hand. It lives in `<bare>/ivar-hooks/` and is selected per worktree
via worktree-local `core.hooksPath`, so a project's own hook manager (husky and
similar, which write `core.hooksPath` into the shared config during
`pnpm install`) does not displace it, and feature worktrees keep the project's
hooks. Because it is worktree-local, it binds the default-branch worktree only.

`ivar deliver` commits onto the default branch by design when landing with the
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

- **Shell commands that write files.** Not classified, by the reasoning above.
- **Repositories outside the hall.** Protection is installed by `ivar sync` on
  repos declared in `ivar.json`. A clone made by hand elsewhere has none of it.
- **In-place edits to existing files**, per the root-only read-only guard.
- **`git commit --no-verify`,** typed by a human or emitted by an agent. Git
  offers no way for a hook to refuse this.

None of this is a security boundary. It is a guardrail against plausible
mistakes — an agent committing to `main` because that is where it happened to
be — and it is built for a hall where the person running it wants the guardrail.
It will not stop a determined process running as your user.
