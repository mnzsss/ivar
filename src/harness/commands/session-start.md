---
description: Start or resume an ivar session — select a feature or explore read-only.
argument-hint: [feature-name]
---

# Session Start

`/ivar-session-start` starts or resumes an ivar session — select a feature or
explore read-only. A session bound to a feature carries everything a fresh
agent needs to continue the feature's work: the promoted repos, the active
plan, and instructions for re-deriving where the feature is in the SPDD cycle.

## Before it opens the session

There is **no health gate**: `ivar session start` does not consult hall health
and never refuses on it. The hall's health ladder
(`uninitialized` / `operational` / `stale` / `degraded`) is what `ivar status`
renders, and reading it is your call, not the command's. `ivar repo pull`
catches up a stale hall; `ivar sync` rebuilds a degraded one; `ivar doctor`
explains either.

What session start *does* do first is a **Smart Fetch**: every registered
repo's read-only default-branch worktree is fetched and fast-forwarded before
the view dir is materialised. Promoted feature worktrees are never touched.
The sweep is best-effort per repo — a repo it could not refresh warns
(`session.smart_fetch_failed` / `session.smart_fetch_skipped`) and the session
still opens.

## Steps

1. Run `ivar feature list` to see existing features.
2. If `$ARGUMENTS` is provided, use it as the feature name. Otherwise, ask the
   user:
   - Pick an existing feature, OR
   - Create a new feature (run `ivar feature create <name>`), OR
   - **Discovery session** — delegate to `/ivar-discovery` for guided
     exploration.
3. Run the appropriate command with optional flags:
   - Feature session: `ivar session start <feature>`
   - Specify a provider: `ivar session start <feature> --provider opencode`
   - Resume an existing session: `ivar session start <feature> --resume`
   - Create without launching a provider: `ivar session start <feature> --detached`
   - Discovery session: **run `/ivar-discovery` instead** — it handles the
     guided discovery flow with optional conversion to a Feature session.
4. Parse the output for the three stable machine-readable binding keys:
   ```
   IVAR_SESSION_ID=<uuid>
   IVAR_FEATURE=<feature-or-empty>
   IVAR_SESSION_PATH=<absolute-view-dir>
   ```
5. Export env vars for this shell session:
   ```bash
   export IVAR_SESSION_ID=<id>
   export IVAR_FEATURE=<feature>
   export IVAR_SESSION_PATH=<path>
   ```
6. From this point forward, **all file reads, writes, and shell commands must
   operate inside the session path**. Promoted repos are mounted directly at
   the view dir's own root — prefix repo paths with
   `$IVAR_SESSION_PATH/<repo>/`. The feature's plan is reachable at
   `$IVAR_SESSION_PATH/plans/<feature>/` (it resolves to the hall's committed
   plan directory; edits there land in the hall). When running shell commands,
   `cd $IVAR_SESSION_PATH` first.

## Continuing an existing feature

A feature session's instruction file (`CLAUDE.md` / `AGENTS.md` at the view
dir root) carries the hall's standing instructions plus a session bootstrap
block. At the start of every conversation, re-derive where the feature is:

1. Run `ivar plan status plans/<feature>/plan.md`.
2. Read the plan artifacts that exist under `plans/<feature>/`.
3. Continue from the first approval gate that is `pending` or
   `needs-revision`.

This is what lets a relay — or a fresh conversation on an existing session —
pick the feature's work back up.

## Important

- **Always use the CLI commands.** Never create `.ivar/features/`,
  `.ivar/sessions/`, state files, or lock files manually. The schema is strict
  — wrong field names cause silent failures.
- **Before creating a new session**, check whether one is already live:
  `ivar session connect --feature <feature>` connects when exactly one is, and
  names every candidate when more than one is. (No command lists sessions, and
  `ivar status` reports repo health only.) If one already exists for the
  feature, use `/ivar-session-connect` instead.
- Multiple sessions may bind the same feature at once (e.g. one per provider);
  they share that feature's worktrees.
- If a repo is read-only, you cannot write to it. Run `/ivar-promote <repo>`
  first.
- Session state is ephemeral. When you stop the session, the view dir is
  deleted — including the projected plan link and the session instruction
  file, which are per-session views, never copies. The worktrees, the plan
  and the feature state persist.
- For discovery sessions, always use `/ivar-discovery` — it provides the
  guided workflow and supports one-way Feature conversion.
