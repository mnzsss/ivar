---
description: Reconnect to an active ivar session instead of creating a new one.
argument-hint: [session-id-prefix]
---

Reconnect to an active ivar session.

## Steps

1. Determine the target session. The user provides either:
   - A session ID prefix: `ivar session connect <id-prefix>`
   - A feature name: `ivar session connect --feature <feature>`
2. Run the command. It emits three stable machine-readable binding keys:
   ```
   IVAR_SESSION_ID=<uuid>
   IVAR_FEATURE=<feature-or-empty>
   IVAR_SESSION_PATH=<absolute-view-dir>
   ```
3. Export the env vars:
   ```bash
   export IVAR_SESSION_ID=<uuid>
   export IVAR_FEATURE=<feature-or-empty>
   export IVAR_SESSION_PATH=<absolute-view-dir>
   ```
4. **Change working directory immediately:**
   ```bash
   cd $IVAR_SESSION_PATH
   ```
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

   All file operations must use paths relative to $IVAR_SESSION_PATH/repos/<repo>/.
   Never read or write files outside the session path.
   When reading files from read-only repos, prefer to search only in promoted repos first.
   ```

7. From this point forward, **all file reads, writes, and shell commands must
   operate inside `$IVAR_SESSION_PATH`**. When running shell commands,
   `cd $IVAR_SESSION_PATH` first.
8. When the agent needs to read context (CLAUDE.md, AGENTS.md), prefer the
   session directory's AGENTS.md over hall-root files.

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
- If the session dir no longer exists (stale), run `ivar session prune` to
  clean up, then `/ivar-session-start`.
