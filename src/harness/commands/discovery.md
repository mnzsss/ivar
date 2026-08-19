---
description: Guided discovery session — explore the codebase, understand a problem, and optionally create a Feature.
argument-hint: [objective]
---

# Discovery

`/ivar-discovery` runs a guided Discovery Session. Use this when you need to
understand a problem, explore the codebase, or make a decision before committing
to a Feature.

## Behavior

- **No active session**: starts a new Discovery Session and begins the guided
  discovery.
- **Active Discovery Session**: continues the current discovery — re-running
  `/ivar-discovery` does not start over.
- **Active Feature Session**: rejects — conversion is irreversible and cannot
  target a Feature session.

## Steps

1. **Check active session**
   - `$IVAR_SESSION_ID` is the signal: no command lists sessions, and
     `ivar status` reports the hall's repo health, not its sessions.
   - If `$IVAR_SESSION_ID` is set and its session has no feature → continue the
     existing discovery.
   - If `$IVAR_SESSION_ID` is set and its session is feature-bound → refuse:
     "This session is already a Feature session. Conversion is irreversible.
     Start a new Discovery Session if needed."
   - If no session exists → proceed to step 2.

2. **No active session — start Discovery**
   - If `$ARGUMENTS` is provided, treat it as a provisional objective.
   - If no `$ARGUMENTS` is provided, ask the user: "What would you like to
     understand or decide?"
   - Run `ivar session start` with no feature argument to create a Discovery
     Session (a session with no feature).
   - Parse output for `IVAR_SESSION_ID`, `IVAR_SESSION_PATH`.
   - Export the env vars.

3. **Discover**
   - Ask one question at a time. Never ask multiple questions in a single turn.
   - Read files, search the codebase (each repo is mounted at the session
     path's own root, as `<name>/` — there is no `repos/` level), run
     read-only commands.
   - **Never write to repos during discovery.** All repos are read-only.
   - Build understanding of:
     - Problem or opportunity
     - Desired outcome
     - Initial scope boundaries
     - Likely affected repos
     - Risks and blocking open questions

4. **Conversion readiness check**
   Once you have identified all of the following, you may offer conversion:
   - Problem or opportunity statement
   - Desired outcome
   - Initial scope boundaries
   - Likely affected repos
   - Risks and blocking open questions

   Open questions may remain if recorded, and the user explicitly confirms they
   want to proceed.

   **Never convert automatically** — always ask the user explicitly.

5. **Offer conversion**
   - Ask: "Ready to convert this Discovery Session into a Feature Session? This
     is irreversible."
   - If yes:
     1. Ask: "Create a new Feature or bind to an existing one?"
        - **New Feature**: ask for the feature name.
        - **Existing Feature**:
          - Run `ivar feature list` to show available features.
          - When the user picks one, check if any repos are already promoted:
            - Run `ivar feature status <feature>`.
            - Show which repos are already promoted (they will become
              immediately writable).
            - Other repos remain read-only until promoted.
     2. Confirm the choice with the user.
     3. For the discovery brief, write a structured summary, omitting
        irrelevant sections.
     4. If `discovery.md` already exists for this feature, ask:
        - **Amend** (default): append a dated section with Session ID, new
          evidence, and changed understanding.
        - **Merge** (explicit): integrate and reorganize. Show the proposed
          document first and get confirmation.
     5. Run `ivar session convert <session-id> <feature>` to bind the discovery
        session to the feature.
     6. Parse the output. Export the binding env vars.
     7. After successful conversion, check if `/ivar-plan` is installed (look
        in `.claude/commands/` or `.opencode/commands/` for `ivar-plan.md`):
        - If installed, offer: "Would you like to create a plan for this
          Feature? Run `/ivar-plan`."
        - Do not run `/ivar-plan` automatically.

## Important

- **Never write to repositories during discovery.** Every repo is read-only
  until promoted.
- **Never convert automatically.** Always get explicit user confirmation.
- **Conversion is irreversible.** A Discovery Session converts exactly once. A
  Feature Session can never go back to discovery.
- **One question at a time.** Build understanding incrementally.
- **Use the CLI.** Never create or modify session files, state.json, or feature
  files manually.
