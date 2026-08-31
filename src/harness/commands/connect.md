---
description: Attach to a feature's session, creating one when none is free, and work inside it.
argument-hint: <feature-name>
---

# Connect

You are executing `/ivar-connect`. **Run step 1 verbatim, before any other
command.** Do not list sessions or inspect `.ivar/` first, and do not run
`ivar session connect` with no arguments — it has nothing to search with and
refuses.

## Steps

1. Run exactly this, substituting nothing but the feature name:

   ```bash
   ivar session connect --feature $ARGUMENTS --create --json
   ```

   `--create` attaches to the feature's most recent session that no agent is
   running in, and starts a fresh one when every candidate is busy or none
   exist. So this command always ends with you inside a session. `ivar session
   start` and `ivar session stop` belong to ivar's own lifecycle; you never
   call them.

   When `$ARGUMENTS` is empty, ask which feature to connect to — never guess.

2. Read the three binding keys out of the JSON:

   ```
   IVAR_SESSION_ID=<uuid>
   IVAR_FEATURE=<feature>
   IVAR_SESSION_PATH=<absolute-view-dir>
   ```

   Do not execute any export statements: the agent session environment is
   already injected, and `ivar session env` resolves it.

3. Work inside `IVAR_SESSION_PATH` from here on.

   > **IMPORTANT FOR AI AGENTS:** your bash tool does NOT persist `cd` across
   > turns. For EVERY subsequent bash call, set the `workdir` parameter to
   > `$IVAR_SESSION_PATH`, or use absolute paths under it.

4. Discover promoted repos with `ivar feature status "$IVAR_FEATURE"`.

5. Emit this block as your response — replace the placeholders with real
   values:

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

6. Prefer the session directory's `CLAUDE.md` / `AGENTS.md` over hall-root
   files — it carries the hall's standing instructions plus the session
   bootstrap block.

## Continuing the feature's work

The plan lives at `$IVAR_SESSION_PATH/plans/<feature>/`. At the start of every
conversation, re-derive where the feature is in the SPDD cycle:

1. Run `ivar plan status plans/<feature>/plan.md`.
2. Read the plan artifacts that exist under `plans/<feature>/`.
3. Continue from the first approval gate that is `pending` or
   `needs-revision`.

## Troubleshooting

- **`session.not_found` naming discovery sessions.** A session started without
  a feature lives under `.ivar/sessions/` and a `--feature` search cannot see
  it — but it may already hold this feature's work. Bind it with
  `ivar session convert <session-id> $ARGUMENTS` rather than starting a second
  session. Ask the user which one to convert; do not pick for them.
- **Connect re-materialises the view dir** on every run, repairing whatever
  drifted or predates the current `ivar`: repo symlinks, read-only guards, the
  provider's config dir and commands, the projected plan link, and the
  session's bootstrap instructions. Running it again is safe and cheap.
- **Each session reconnects under the provider that opened it.** One relayed to
  OpenCode comes back as an OpenCode session, not as the hall's default.
