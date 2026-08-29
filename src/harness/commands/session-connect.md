---
description: Reconnect to an active ivar session instead of creating a new one.
argument-hint: [feature-name]
---

# Session Connect

You are executing `/ivar-session-connect`. Do the steps below now — do not
describe them. Reconnect to the feature's active session, export the emitted
bindings, and change into the view dir.

Connect re-materialises the session's view dir, repairing anything that drifted
or predates the current `ivar`: repo symlinks, read-only guards, the provider's
config dir and commands, the projected plan link, and the session's bootstrap
instructions.

## Steps

1. Determine the target feature from `$ARGUMENTS`. When it is empty, ask which
   feature to reconnect to — never guess. The value is a feature name; run:
   `ivar session connect --feature $ARGUMENTS`
   If it names multiple candidate sessions, ask the user which one to connect to.
2. Run the command. It emits three stable machine-readable binding keys:
   ```
   IVAR_SESSION_ID=<uuid>
   IVAR_FEATURE=<feature-or-empty>
   IVAR_SESSION_PATH=<absolute-view-dir>
   ```
3. Note that `session connect` prints export statements for human terminals, but the agent session environment is already injected by `ivar session start` (or resolvable via `ivar session env`). Do not execute export statements manually.
4. **Change working directory immediately:**
   ```bash
   cd $IVAR_SESSION_PATH
   ```

   > **IMPORTANT FOR AI AGENTS:** Your bash tool does NOT persist the `cd` command across turns. For EVERY subsequent bash tool call in this session, you MUST set the `workdir` parameter to `$IVAR_SESSION_PATH` or use absolute paths.

5. If the session is feature-bound, discover promoted repos via
   `ivar feature status "$IVAR_FEATURE"`.
6. Emit a guard instruction as the response to the user — print exactly this
   block (replace placeholders with actual values):

   ```
   ## Session guard

   Connected to session <id> on feature <feature>.
   Session path: $IVAR_SESSION_PATH

   **Promoted repos (editable):** <list of promoted repo names>
   **Read-only repos (do NOT edit or load context from):** <list of read-only repo names>

   All file operations must use paths relative to $IVAR_SESSION_PATH. Repos are
   mounted directly at the view dir's root (`$IVAR_SESSION_PATH/<repo>/`); the
   feature's plan is at `$IVAR_SESSION_PATH/plans/<feature>/`.
   Never read or write files outside the session path.
   When reading files from read-only repos, prefer to search only in promoted repos first.
   ```

7. From this point forward, **all file reads, writes, and shell commands must
   operate inside `$IVAR_SESSION_PATH`**. When running shell commands, set the
   tool's `workdir` to `$IVAR_SESSION_PATH` (do not rely on `cd`, which does
   not persist between calls).
8. When the agent needs to read context (CLAUDE.md, AGENTS.md), prefer the
   session directory's file over hall-root files — it carries the hall's
   standing instructions plus the session bootstrap block.

## Continuing the feature's work

A connected feature session carries its plan at
`$IVAR_SESSION_PATH/plans/<feature>/` and its bootstrap instructions in the
session's `CLAUDE.md` / `AGENTS.md`. At the start of every conversation,
re-derive where the feature is in the SPDD cycle:

1. Run `ivar plan status plans/<feature>/plan.md`.
2. Read the plan artifacts that exist under `plans/<feature>/`.
3. Continue from the first approval gate that is `pending` or
   `needs-revision`.

## When to use

- Resuming work after the agent was restarted or a new conversation started.
- The session view dir already exists and you want to continue where you left
  off.
- Prefer this over `/ivar-session-start` when a session is already active.

## Important

- **Do not create a new session if one already exists for your feature.** Use
  this command instead.
- Both `session start` and `session connect` emit the same three binding keys
  (`IVAR_SESSION_ID`, `IVAR_FEATURE`, `IVAR_SESSION_PATH`).
- Connect re-materialises the view dir for the **session's own provider** (the
  one recorded in the session, or the hall's default for a legacy session),
  so a session relayed to OpenCode reconnects as an OpenCode session.
- If connect refuses with `session.not_found`, the session's view dir is gone
  and there is nothing to reconnect to — start a fresh one with
  `/ivar-session-start`. `ivar session prune` is not the repair for that: it
  removes view dirs that still **exist** but hold no readable `state.json`,
  and a dir that is already gone is invisible to it.
