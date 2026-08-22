---
description: Execute an approved plan with provider-native coordination and an Ivar Run Receipt.
argument-hint: <plan-path>
---

# Execute

`/ivar-execute` executes the approved plan at `$ARGUMENTS` for the current
Feature Session. The active provider coordinates its own native subagents;
Ivar records the provider-neutral Run Receipt lifecycle and final evidence.

## Prerequisites

- You must be inside a **Feature Session**: `IVAR_FEATURE` and
  `IVAR_SESSION_ID` must be set, and `$IVAR_SESSION_PATH/state.json` must be
  readable.
- The feature must exist in the hall (`ivar feature list`).
- The plan file must be accessible and all three planning gates — Requirements,
  Analysis, and Plan — must be approved.

## Steps

1. Read `$ARGUMENTS`. In a feature session it is normally available at
   `plans/<feature>/plan.md` relative to `$IVAR_SESSION_PATH`. Verify its gates:

   ```sh
   ivar plan status $ARGUMENTS
   ```

2. Inspect the current Run Receipt before starting work:

   ```sh
   ivar feature execute status $IVAR_FEATURE
   ```

   - With no receipt or a terminal receipt, begin a new run:

     ```sh
     ivar feature execute start $IVAR_FEATURE --plan $ARGUMENTS
     ```

   - For an `active` or `blocked` receipt, continue the logical run with:

     ```sh
     ivar feature execute start $IVAR_FEATURE --plan $ARGUMENTS --resume
     ```

     This may attach the current provider to a run begun by another provider;
     do not claim continuity of a provider conversation.

   - To abandon a non-terminal receipt and begin again, use:

     ```sh
     ivar feature execute start $IVAR_FEATURE --plan $ARGUMENTS --restart
     ```

   - For a `diverged` receipt, inspect the approved revision. Adopt it only
     after confirming it remains the intended scope:

     ```sh
     ivar feature execute accept-revision $IVAR_FEATURE --plan $ARGUMENTS
     ```

     Then resume, or restart if the revision requires a fresh execution.

3. Act as the coordinator. Decompose the approved plan using this provider's
   native subagent capabilities. Schedule tasks concurrently only when they
   are independent and do not conflict. Monitor and synthesize their results
   using the provider's own facilities; do not persist provider-native child or
   conversation identifiers in Ivar.

4. Ask the human directly when a decision needs their input. If newly
   discovered work is outside the approved plan and can be isolated, create a
   child Feature rather than silently expanding this run. Record the follow-up
   in the final report.

5. Verify the completed work. Write a temporary JSON report with the required
   `summary`, `tasks`, and `verification` fields. Each task needs `title`,
   `status`, and `result`; each verification needs `command`, `status`, and
   `summary`. Optional fields are `agents`, `deviations`, `blockers`, and
   `follow_ups`. Use provider-neutral role names and concise evidence, never
   transcripts or provider-native identifiers.

6. Finish the receipt with the appropriate outcome:

   ```sh
   ivar feature execute finish $IVAR_FEATURE --plan $ARGUMENTS \
     --report-json /tmp/ivar-run-report.json --outcome succeeded
   ```

   Valid outcomes are `succeeded`, `failed`, and `blocked`. A blocked or failed
   outcome must explain the relevant blocker in the report.

7. Inspect the finished receipt and communicate its outcome and verification to
   the human:

   ```sh
   ivar feature execute status $IVAR_FEATURE
   ```
