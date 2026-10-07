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

## Execution sandboxing and provider control

On Linux (kernel >= 5.13 with Landlock support), Ivar applies a kernel-enforced
write sandbox (`Landlock`) to the provider process before running it. Filesystem
writes outside the session's writable set (the view directory, feature directory,
descendant feature directories, and promoted worktrees) are denied directly by the
kernel, even from arbitrary shell commands and grandchild subprocesses. For feature
sessions, directory grants on `.ivar/features/` and promoted repo directories enable
dynamic child feature creation and promotion mid-session. The cost: Bash inside a
feature session can, at the kernel level, write any feature's record under
`.ivar/features/` and any worktree of those repos, including the read-only
default-branch checkout, whose write bits are cleared on its root only. Edit and Write
stay limited to the feature's subtree by `ivar guard`. On non-Linux platforms (such as
macOS) or older kernels, Ivar runs without kernel sandboxing and relies on the advisory
tool guard.
Ivar does not control or schedule native provider subagents. The active provider
creates, schedules, monitors, and synthesizes subagents. The Run Receipt records exact
baseline and final snapshot evidence for audit, but it neither attributes individual
writes nor reverts them.

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

## Session write guard: Enforcing (Linux) vs Advisory (Everywhere)

Protection operates in two distinct tiers:

1. **Kernel write enforcement (Linux):** On Linux systems supporting Landlock,
   `ivar session start` launches the provider process under an irreversible
   write-only Landlock sandbox. Any attempt to modify, create, or delete files
   outside the session's allowed roots fails with `EACCES` (`Permission denied`)
   at the syscall level.
2. **Advisory tool guard (All platforms):** The `ivar guard` hook intercepts
   structured tool writes (Write, Edit, ApplyPatch, device writes) before execution.
   Its primary role is **diagnostic legibility**: when an agent attempts an illegal
   write, the guard returns the session's current `writable set: ...` so the agent
   understands why the path is disallowed and can ask for repo promotion, rather
   than receiving an opaque OS permission error. When no session resolves from the
   cwd or the target path, the guard lists the scratch directories of live sessions
   belonging to the target feature if any, or states the count of live sessions in the hall.

### Multi-target structured writes and device tools

The guard inspects all targets affected by a single tool invocation:
- **Multi-target structured writes:** Hashline section headers (`[path#tag]`), `MV` destinations, and apply-patch headers (`*** Add File:`, `*** Update File:`, `*** Delete File:`, `*** Move to:`) are extracted from `args.input`. A multi-target write is allowed only if every target is allowed; otherwise, the denial names the first disallowed target.
- **Device and direct tools:** Writes to `xd://ast_edit` or direct `ast_edit` tool invocations inspect the literal directory prefix of each path pattern in `paths[]`. Writes to `xd://lsp` or direct `lsp` tool invocations with mutating actions (`rename`, `rename_file`, `request`, or `code_actions` with `apply: true`) check the target file (and destination path for `rename_file`). Read-only actions and dry renames (`apply: false`) are allowed. Unparseable device content fails closed (denied).
- **LSP rename residual:** An allowed `xd://lsp` rename can rewrite symbol references in files outside the session's writable set on platforms without Landlock enforcement.

On platforms without Landlock (e.g. macOS), the advisory hook is the primary line of
defense for structured tools. Its effectiveness depends on the provider honouring the
hook protocol:
- **Claude Code:** The guard exits 0 with `permissionDecision: deny` in the JSON body.
- **OpenCode:** The guard exits non-zero to signal tool rejection.
- **OMP:** The guard exits non-zero with the denial reason on stdout.

On an allow, the guard's output also carries the repository instructions for the
touched paths: `additionalContext` in the JSON body for Claude Code, stdout for
OMP and OpenCode. A deny never carries them.

## Repository instructions in sessions

`ivar guard` delivers the linked repos' own instruction files on the path of each
tool call (see [Concepts](../concepts.md)). What it can and cannot see:

- **Touched paths come from the tool input.** File-path fields, the working
  directory, and the words of a shell command: after `&&`, `||`, `;`, `|`, `&`,
  parentheses and newlines, the target of `cd` (followed for later words),
  `--opt=path`, `-C path`, and glob words expanded through the view dir (at most
  50 matches). Words containing `$` or a backtick are skipped, so a path the shell
  builds at run time is not seen; the pointer list in the session's instruction
  file is the fallback.
- **Claude Code inlines about 10,000 characters of context per hook.** The guard
  entry and five slice entries in `.claude/settings.json`
  (`ivar guard --provider claude-code --slice 1` to `--slice 5`) each carry
  9,500 characters of one context computed once per tool call, so up to 57,000
  characters per call arrive whole. Beyond that, the context is cut and the last
  slice names only the files the cut reached, so the agent can read them. Those
  files are not marked as received, so a later call sends them again. A slice
  entry never decides: a deny from the guard entry still blocks the call.
- **Halls must run `ivar sync` after upgrading ivar.** The slice entries live in the
  hall's `.claude/settings.json`, which `ivar sync` rewrites. A hall synced by an
  older ivar keeps the old hooks until then. A session's view links that file, so
  the next tool call after the sync picks up the new hooks.
- **Claude Code's main agent is keyed by its session id.** Instructions that
  `/compact` or `/clear` removes from the conversation are not sent again until
  the file changes, because the guard still records them as received.
- **The glob cap limits matches, not the directory read.** A glob word in a shell
  command is expanded by reading the directories it names. At most 50 matches
  count as touched, but a large directory is still read in full to find them.
- **OpenCode's `glob` does not traverse the view dir's repo symlinks** (`app/**`
  finds nothing), so a model that globs first may conclude a repo is empty. Reads
  and shell commands still deliver the instructions.
- **OpenCode's default `researcher` model ignores long instruction context.** In
  live evaluation it answered from a 24 KB instruction file 0 times out of 3 even
  though the whole file was delivered; `router/implementer` answered 2 out of 2.
  That is model quality, not delivery.
- **Delivery state lives in the view dir**, at
  `<view>/<config dir>/ivar/instructions/`, the only place the guard can write
  under the OMP Landlock sandbox. It dies with the view dir, so a new session
  receives every file again.

## Provider-specific capabilities and limitations

- **Session resume:** Claude Code supports resuming prior sessions (`--resume`).
  OpenCode and OMP do not support session resume; `ivar session start --resume`
  will reject attempts to resume with these providers.
- **MCP authentication scope:** OMP credentials installed via `ivar mcp auth` are
  scoped to the active omp profile (`OMP_PROFILE` or `PI_PROFILE`, defaulting to
  `default`). Switching profiles requires re-authenticating for that profile.
  However, credentials do not require manual re-authentication on expiry: omp
  natively refreshes tokens using the rendered `auth` block in `.omp/mcp.json`.
  omp reads `.omp/mcp.json` ahead of `opencode.json`'s `mcp` entries, so the
  per-hall `credentialId` is the one it uses.
- **MCP authentication inspection and multi-provider auth:**
  - **Claude Code:** `ivar mcp status` reads `~/.claude/.credentials.json` directly. When Claude stores credentials in the OS Keychain (such as on macOS), or when credentials cannot be decrypted from disk, local state is reported as `unknown`. Use `ivar mcp status --live` to inspect runtime connectivity via `claude mcp list`.
  - **Harness live inspection (`--live`):** `--live` parses plain-text output from `claude mcp list` and `opencode mcp list`. If the underlying harness changes its human-readable status formatting, unrecognized lines fail closed to `unknown` rather than guessing. Note that `--live` invokes `claude mcp list`, which connects to every declared server and can take several seconds.
  - **OMP:** OMP provides no live status command; `ivar mcp status` (with or without `--live`) determines credentials by running `omp token` and marks the source as `local`.
  - **Doctor diagnostic:** `ivar doctor` diagnoses missing MCP credentials using offline local store checks only (it never runs `claude mcp list` or `opencode mcp list`); for OMP, that local check runs `omp token`.
  - **Multi-provider authorization (`--all-providers`):** `ivar mcp auth --all-providers` runs authentication for each available provider independently, including providers that are already authenticated; it never copies or shares tokens between providers (one grant each). After completing all legs, ivar performs a post-run dropped-grant check (`mcp.grant_dropped`) when two or more providers ran, warning if a previously granted leg lost its authorization due to subsequent provider logins.
  - **Credential replacement:** `ivar mcp auth` always replaces an existing credential.
  - **OMP upgrade:** after upgrading ivar, omp users run `ivar mcp auth <server> --provider omp` once in each hall; until then omp reports the server as missing.

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
- **Repositories outside the hall.** Protection is installed by `ivar sync` on
  repos declared in `ivar.json`. A clone made by hand elsewhere has none of it.
- **`git commit --no-verify`,** typed by a human or emitted by an agent. Git
  offers no way for a hook to refuse this.
None of this is a security boundary. It is a guardrail against plausible
mistakes — an agent committing to `main` because that is where it happened to
be — and it is built for a hall where the person running it wants the guardrail.
It will not stop a determined process running as your user.

## Codebase graph indexing and plain-text bounds

The codebase graph indexes all Git-tracked UTF-8 text files, but applies clear operational bounds:

- **Searchable content bound:** Each file is indexed up to a maximum of 256 KiB of UTF-8 content.
  Content beyond this bound is not indexed for full-text search, and the file record marks
  `content_truncated: true`. Exact requested-file retrieval still reads the full current file from disk.
- **Binary and unparseable files:** Files containing NUL bytes or invalid UTF-8 within the content probe
  are rejected as binary and skipped without failing the repository index.
- **Excluded files:** Untracked files, Git-ignored files, and ignored build/dependency directories
  (such as `target/`, `node_modules/`, `.git/`, `.ivar/`) are not indexed.
- **Plain-text files without grammars:** Text files lacking tree-sitter grammars participate in path,
  basename, and full-text search, but do not produce synthetic AST symbols, callers, callees, or
  call graph edges.
- **Response budgets and truncation:** Exploration outputs enforce deterministic budgets (18,000 characters
  for standard exploration; 24,000 for explicit file requests). When files or sources exceed budgets,
  omitted files are listed under `not_shown` with an exact JSON `paths` continuation call.
