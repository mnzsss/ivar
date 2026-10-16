---
name: ivar-explore
description: Start or continue a guided Discovery Session when the user describes a problem, idea, decision or change that is not scoped yet and needs understanding before work begins. Do not use it for a plain question about existing code, and do not use it inside a Feature Session.
---

# Explore

`ivar-explore` runs a guided Discovery Session: understand a problem, explore
the code, or settle a decision before committing to a Feature. The user does
not invoke it; load it when their request is a problem, idea, decision or
change that is not scoped yet. Answer a plain question about existing code
directly and open no session for it.

## Behavior

- **No active session**: starts a new Discovery Session and begins the guided
  discovery, without asking for confirmation.
- **Active Discovery Session**: continues the current discovery and
  never starts a second one.
- **Active Feature Session**: does nothing but explain — conversion is
  irreversible and cannot target a Feature session.

## Steps

1. **Check the active session**
   - `$IVAR_SESSION_ID` is the signal: no command lists sessions, and
     `ivar status` reports the hall's repo health, not its sessions.
   - If `$IVAR_SESSION_ID` is set and its session has no feature → continue the
     existing discovery.
   - If `$IVAR_SESSION_ID` is set and its session is feature-bound → stop and
     say: "This session is already a Feature session. Conversion is
     irreversible. Start a new Discovery Session if needed."
   - If no session exists → step 2.

2. **Start the Discovery Session**
   - Treat the user's message as the provisional objective.
   - Run `ivar session start` with no feature argument.
   - Parse the output for `IVAR_SESSION_ID`, `IVAR_SESSION_PATH` and export them.

3. **Discover**
   - Ask one question at a time. Never ask multiple questions in a single turn.
   - Use `ivar graph explore <query>` (or `ivar graph explore <query> --repo <repo>`) to inspect likely symbols, entry points, and relation evidence before broad file reading.
   - Graph evidence is advisory: if a graph query is missing, stale, empty, unsupported, or fails, fall back directly to source search and file reading.
   - Read files, search the codebase (each repo is mounted at the session
     path's own root, as `<name>/` — there is no `repos/` level), run
     read-only commands.
   - **Never write to repos during discovery.** All repos are read-only.
     Hall files outside `.ivar/` (docs, `HALL.md`, harness config) are
     writable, so hall docs can be updated without a feature. The hall's
     git hooks and config, provider hook and MCP config, and `.env` stay
     protected.
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
     1. Write the discovery brief through the CLI. The doc's frontmatter is
        ivar-owned — `name`, `status`, `created_at`, `updated_at` and
        `sessions` — so a hand-written file has no `sessions` entry and
        conversion will refuse it.
        - Create the doc if it does not exist:
          `ivar discovery create <name> [--title <title>]`. The doc is written
          into the discovery session's view dir (`discovery.md`). The name is the
          unit of work's name, lowercase kebab-case; it becomes the feature
          name at conversion.
        - Write the brief:
          `ivar discovery amend <name> --file <path>`.
          `--file -` reads stdin. Append is the default and records the
          session in `sessions`, which is what conversion looks for.
        - Use `--merge` only to replace the whole document; it requires
          `--expected-hash <sha256>`, the hash `ivar discovery show` reports.
          Show the proposed document and get confirmation first.
     2. Confirm the brief with the user.
     3. Run `ivar session convert <session-id>` to bind the session. The
        command moves `discovery.md` into `.ivar/features/<name>/discovery.md`
        and creates the feature when it does not exist yet. Do not ask the user
        to choose a new or existing feature — the name follows from the brief,
        and `ivar feature create` beforehand is unnecessary.
     4. Parse the output. Export the binding env vars.
     5. After successful conversion, ask: **(a) Plan it now**, **(b) Stop here**.
        Load the `ivar-plan` skill only on (a). `ivar sync` ships `ivar-plan` to
        every provider at the hall root, so do not look for it under the session
        directory.

## Important

- **Never write to repositories during discovery.** Every repo is read-only
  until promoted.
- **Never convert automatically.** Always get explicit user confirmation.
- **Conversion is irreversible.** A Discovery Session converts exactly once. A
  Feature Session can never go back to discovery.
- **One question at a time.** Build understanding incrementally.
- **Use the CLI.** Never create or modify session files, state.json, or feature
  files manually.
