# ADR-0003 — Env by process, guard by hook

- **Status:** accepted
- **Date:** 2026-08-29

## Context

The session environment (`IVAR_HALL`, `IVAR_SESSION_ID`, `IVAR_SESSION_PATH`,
`IVAR_PROVIDER`, `IVAR_FEATURE`) must be present when an agent runs inside a
view dir, and structured writes outside the session's writable set must be
denied.

Two providers are in scope — Claude Code and OpenCode — and they differ in
what a hook can do:

| | Claude Code | OpenCode |
|---|---|---|
| Env injection from hook | No (`settings.json.env` is static) | Yes (`shell.env` injects on every shell) |
| Hook exit code | Always 0; decision travels in JSON body | 0 = allow, non-zero = deny |
| Hook event | `PreToolUse` (permission model) | `pre-tool-call` (before execution) |

## Decisions

### D1 — Session resolved by disk walk-up first, the agent's own id second

`SessionEnv::resolve_by_cwd` walks from the current directory upward looking
for `state.json` inside a directory whose name is a valid `SessionId`. No
environment variable is read. This means the session contract is a
filesystem fact, not an ambient one — a process that happens to have the
right `IVAR_SESSION_ID` in its environment but is not actually inside the
view dir will not resolve.

The guard and the graph use `SessionEnv::resolve_for_agent`: the same
walk-up, but when it finds no view dir, an `IVAR_SESSION_ID` naming a live
session of the cwd's hall wins over the promoted-worktree fallback. A
subfeature's view links its parent's feature worktree, so a child agent's
canonical cwd lands there; the fallback would hand it the parent's session
and the parent's writable set.

For a write, the guard uses `SessionEnv::resolve_for_write`, which inserts
one step after the walk-up: when the cwd lies in no view dir, the live
session whose view dir holds the write's first absolute target wins over
the ambient id. The path names the session it writes into. The walk-up
still comes first, so an agent standing in one session's view dir never
borrows another session's writable set, and a session of another hall
never matches.

### D2 — Env injected by the process, guard enforced by hook

`SessionEnv` is built once in `build` and applied to the provider command at
spawn time (`session start`). The hook or plugin only verifies and blocks —
it never injects environment variables. This is the "env by process, guard by
hook" separation: the process guarantees the environment; the hook enforces
the write boundary.

### D3 — Provider asymmetry is intentional

Claude Code cannot inject env from a hook because its `settings.json.env`
is static and `SessionStart` re-derives binding independently. OpenCode's
`shell.env` injects env on every shell execution. Rather than paper over
this difference, the guard is designed around it: `ivar guard` is
provider-neutral in its decision logic (`decide`) and provider-specific in
its output shaping (`GuardOutcome`).

## Consequences

- The session contract is testable without a running provider: a directory
  layout with `state.json` is sufficient.
- A hook that fails to deny a write is a non-event for default-branch and
  base-branch worktrees: the filesystem holds them read-only, so the hook is
  the error message, not the barrier. A parent's feature worktree seen by a
  subfeature is the exception — its write bits stay on for the parent — so
  there the hook (and, under the Linux sandbox, Landlock) is the barrier.
- Adding a third provider means writing a new input deserialiser and output
  shaper; the decision logic (`decide`) is shared.
