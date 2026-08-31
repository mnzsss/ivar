---
description: Execute an approved plan with provider-native coordination, marking each wave complete in the plan as it lands.
argument-hint: <plan-path>
---

# Execute

`/ivar-execute` executes the approved plan at `$ARGUMENTS` for the current
Feature Session. The active provider coordinates its own native subagents;
`plan.md` is the progress record, updated at each wave checkpoint.

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

2. Act as the coordinator. Read `plan.md` and `plans/<feature>/tasks/`. Process
   the plan wave by wave using provider-native subagent capabilities:
   - For the current wave, dispatch ONE subagent per task packet in `plans/<feature>/tasks/`.
     Hand the subagent ONLY that task packet's path (`plans/<feature>/tasks/NN-*.md`),
     and instruct it to follow the packet's steps exactly and NOT edit `plan.md`.
   - As each subagent completes its task, fill the result and evidence back into the task packet.
   - Do not persist provider-native child or conversation identifiers in Ivar.
   - After all tasks in a wave pass their exit criteria, pause for the wave checkpoint.
     Summarize the completed wave and request human approval to proceed. Do NOT start
     the next wave until the human approves the wave checkpoint.

3. At the wave checkpoint, after the human approves, mark the wave complete in
   `plan.md` before starting the next wave:
   - In the wave's task table, set each completed task's `Done` cell to `[x]`.
   - Flip each satisfied exit-criteria checkbox from `- [ ]` to `- [x]`.
   - Mark the wave heading complete: `### Wave N — <outcome> ✅`.
   Never edit `plan.md` while a wave is still in progress — only at its checkpoint.

4. Ask the human directly when a decision needs their input. If newly
   discovered work is outside the approved plan and can be isolated, create a
   child Feature rather than silently expanding this run.
