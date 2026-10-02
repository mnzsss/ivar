---
name: ivar-execute
description: Execute an approved feature plan wave by wave with subagent isolation, lightweight validation, dual-axis review barrier, and gated delivery.
argument-hint: [plan-path] [--mode goal|default]
---

# ivar-execute

`ivar-execute` executes an approved feature plan wave by wave. It acts strictly as a coordinator: it dispatches implementation to subagents, runs lightweight validation at wave checkpoints, records progress and deferred validation failures in the run receipt with `ivar feature execute checkpoint`, barriers at post-wave dual reviews (Standards review and Spec review), and facilitates human-gated delivery. Never edit `plan.md` during a run: any edit moves its fingerprint and diverges the run.

## Interactive choice convention

For every question with selectable choices:
- If the harness provides an interactive `Ask` tool (or prompt choice mechanism), use `Ask`.
- If `Ask` is unavailable, present the question textually with equivalent lettered options (`a`, `b`, `c`, ...).
- Questions requiring open-ended input remain plain textual prompts. Never bundle multiple questions into a single prompt.

## Commit and PR text

Never add AI attribution to commit messages, PR titles or PR bodies: no
`Generated with …` line and no `Co-Authored-By:` trailer naming an AI agent.
This overrides any harness default. Pass the rule to every subagent that commits.

## Execution modes

`ivar-execute` operates in one of two modes:
- **`default`**: Guided wave execution with human confirmation at every wave gate, failure choice, review finding selection, and delivery prompt.
- **`goal`**: Autonomous loop running from Wave 1 through `execute finish`, fixing failures within strict caps, and stopping before delivery or integration.

### Mode resolution & receipt state
1. On a new run, `--mode <mode>` is passed to `ivar feature execute start <feature>` (defaulting to `default` if omitted).
2. On resume, read the active mode from `ivar feature execute status <feature> --json` (or `ivar feature execute start <feature> --resume --json`). If `--mode <mode>` was passed on resume, the receipt records the mode change.
3. Detect the coordinator provider from the last entry in `receipt.coordinators[].provider`.

### Caps
| Target | Cap | Behavior when cap reached |
| --- | --- | --- |
| Wave lightweight validation | Max 3 fix attempts per wave | Record failure in checkpoint summary under `Deferred validation failures` and proceed to next wave |
| Post-wave review barrier | Max 3 review-fix rounds | Record remaining open findings in finish report `follow_ups` and proceed to finish |

### Provider behavior
- **`claude-code` provider**:
  Print the ready-to-paste `/goal` line for the user:
  ```text
  /goal Run `ivar feature execute status <feature>` and quote its output: the run is `succeeded` or `failed`, and every wave in plan.md has a `wave <n>:` checkpoint. Both the Standards review and the Spec review report no findings, or 3 review-fix rounds ran and the remaining findings are in the finish report's follow_ups. No wave took more than 3 validation-fix attempts; anything still failing after the third is listed under Deferred validation failures. `ivar feature deliver` and `ivar feature integrate` were not run. Stop there and hand delivery to the human.
  ```
  Replace `<feature>` with the feature name. The prompt is under 4,000 characters and states the exact measurable end state, caps, and delivery stop boundary.
- **`opencode` / `omp` providers**:
  Execute the in-skill loop autonomously using the same termination condition, caps, and stopping boundary.

### Coordinator evidence contract
In goal mode, the coordinator MUST print clear evidence blocks to the transcript so the evaluator or audit trail can verify progress:
- Each wave's lightweight validation command output and pass/fail result.
- Each review round's finding counts for Standards and Spec reviews.
- The final `ivar feature execute status <feature>` output after finishing.

## Execution workflow

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Validate Feature State & Plan                           │
│    ├─ Verify plan approval via ivar feature status --json   │
│    ├─ Verify wave lightweight validation contracts         │
│    └─ Initialize/resume execution run receipt:             │
│         ivar feature execute start <feature> [--mode <m>]  │
│         (pass --plan <plan-path> if not default)           │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 2. Wave Execution Loop (Wave 1 .. Wave N)                  │
│    ├─ Dispatch wave tasks exclusively to subagents          │
│    ├─ Coordinator NEVER edits code directly                 │
│    ├─ Run wave lightweight validation commands              │
│    │    ├─ Pass: auto-checkpoint (goal) or prompt human     │
│    │    └─ Fail: auto-fix <=3 times (goal) or ask human     │
│    │         (if >3 attempts fail in goal: defer failure)   │
│    └─ Record checkpoint: ivar feature execute checkpoint    │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 3. Dual-Axis Review Barrier                                 │
│    ├─ Dispatch isolated Standards review & Spec review      │
│    │  (concurrently if supported by harness)                │
│    ├─ Wait for both review reports (review barrier)         │
│    ├─ Present combined summary with distinct axes           │
│    └─ Review fix loop:                                      │
│         ├─ Goal mode: auto-fix all findings <=3 rounds      │
│         │  (remaining open findings -> finish follow_ups)   │
│         └─ Default mode: ask human which findings to fix    │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 4. Completion & Delivery / Integration Gate                 │
│    ├─ Close active execution run:                           │
│    │    ivar feature execute finish <feature>               │
│    │      --report-json <path> --outcome <outcome>          │
│    ├─ Goal mode STOP: hand delivery/integration to human    │
│    ├─ Default mode branch on is_subfeature:                 │
│    │    ├─ Subfeature: stop; parent session integrates      │
│    │    └─ Root feature: Draft delivery via ivar-deliver    │
└─────────────────────────────────────────────────────────────┘
```

---

### Phase 1: Preparation & plan validation

1. **Locate feature session & plan:**
   - Resolve `$IVAR_FEATURE` and `$IVAR_SESSION_PATH`.
   - Resolve target plan from `$ARGUMENTS` or default to `../../plan.md` relative to `$IVAR_SESSION_PATH`.
   - Parse `--mode <goal|default>` if provided in `$ARGUMENTS`.
2. **Verify plan approval:**
   - Run `ivar feature status <feature> --json` to verify the plan artifact is approved. If not approved, stop and instruct the user to approve the plan first.
3. **Verify wave validation contracts:**
   - Inspect every wave defined in `plan.md`.
   - **Lightweight validation requirement:** Every wave MUST explicitly define lightweight validation commands (such as scoped unit tests, type checks, or targeted linter commands).
   - If any wave lacks explicit lightweight validation commands, stop execution immediately and require the plan to be updated and re-approved before proceeding.
4. **Initialize or resume run receipt:**
   - Run `ivar feature execute start <feature> [--plan <plan-path>] [--mode <mode>]` (or with `--resume [--mode <mode>]` if resuming an existing run).
   - Inspect the returned JSON or run `ivar feature execute status <feature> --json` to determine the active `mode` and coordinator `provider`.
   - If `mode == "goal"` and provider is `claude-code`, print the `/goal` prompt line.

---

### Phase 2: Wave execution loop

Process waves strictly in sequential plan order (Wave 1, then Wave 2, up to Wave N). Never start Wave `K+1` before Wave `K` is checkpointed.

For each wave:

1. **Subagent dispatch:**
   - Dispatch the wave's task packets exclusively to implementation subagent(s).
   - Build every dispatch prompt from `references/subagent.md`, filling each field with absolute paths. A dispatch missing the working directory, worktree root, or artifact paths is incomplete.
   - **Coordinator isolation invariant:** The coordinator NEVER writes or edits feature code directly. All code edits, refactors, and test additions MUST be performed by subagents.
2. **Execute lightweight validation:**
   - Run the explicit lightweight validation commands `plan.md` declares for this wave.
   - When validating changed files across touched repos, query `ivar graph affected <files...>` to discover affected tests and runnable commands. Graph recommendations are advisory: use them to focus verification, and fall back directly to declared wave commands, project test runners, or targeted file execution when graph queries return empty results, fail, or lack runner configuration.
   - **Case A: Validation Passes**
     - **Goal mode:** Print validation output proof to the transcript and proceed immediately to step 4 (Record wave checkpoint) without prompting the human.
     - **Default mode:** Present the wave result, completed tasks, and validation output. Prompt the human for explicit approval to complete the wave.
   - **Case B: Validation Fails**
     - **Goal mode:**
       - Track validation fix attempts for this wave (max 3 attempts).
       - If attempt $\le$ 3: Dispatch fix to an implementation subagent, rerun lightweight validation commands, and evaluate again.
       - If attempt > 3 and validation still fails: Print the failure details, record the failure for this wave's checkpoint summary under "Deferred validation failures", and proceed to step 4.
     - **Default mode:**
       - Report the exact validation failure and command output to the human.
       - Ask the human to choose between:
         - **(a) Fix now:** Dispatch the fix to an implementation subagent, rerun the lightweight validation commands, and return to the checkpoint.
         - **(b) Defer failure:** Keep the failure for this wave's checkpoint summary under "Deferred validation failures". With human approval, allow advancing while carrying the failure into the final review and correction cycle.
4. **Record wave checkpoint:**
   - Append the wave checkpoint to the run receipt:
     ```bash
     ivar feature execute checkpoint <feature> --wave <n> --summary "<completed tasks; exit criteria met; Deferred validation failures: <list or none>>"
     ```
   - Never edit `plan.md` during a run — not task checkboxes, not wave notes, not deferred failures. Task packets and `plan.md` are read-only inputs.
   - If the plan itself must change, stop and ask the human; re-approval followed by `ivar feature execute accept-revision` is their decision.

---

### Phase 3: Post-wave review barrier & correction cycle

Once all planned waves are checkpointed, execute the final dual-axis review:

1. **Dual review dispatch:**
   - Dispatch two isolated review subagents:
     - **Standards review:** Evaluates code against repository coding standards, architectural guidelines, and baseline code smells per repo.
     - **Spec review:** Evaluates implementation against `requirements.md` and `plan.md` across all touched repos.
   - Dispatch reviewers concurrently when supported by the harness, or sequentially if concurrency is unavailable.
2. **Review barrier:**
   - The coordinator MUST wait for both review subagents to complete their reports before presenting results or proposing actions.
3. **Combined report:**
   - Compile both reports into a structured summary, keeping `## Standards` and `## Spec` strictly separated.
   - Include any unresolved Deferred validation failures carried forward from previous waves, read from `ivar feature execute status <feature>`.
4. **Fix selection & revalidation loop:**
   - **Goal mode:**
     - Check if any review findings or carried deferred validation failures exist.
     - If findings exist and review round $\le$ 3:
       - Dispatch all findings and carried failures to an implementation subagent to fix.
       - Rerun both Standards review and Spec review subagents (incrementing the round counter).
       - Repeat the review barrier until 0 findings remain or 3 review-fix rounds have completed.
     - If findings remain after round 3: Record the remaining open findings to be placed in the finish report's `follow_ups` field.
     - Print the final review outcome and finding counts to the transcript. Proceed directly to Phase 4.
   - **Default mode:**
     - Ask the human which reported findings (if any) should be addressed.
     - If findings are selected:
       - Dispatch the selected fixes exclusively to an implementation subagent.
       - Ask the human:
         - **(a) Rerun both reviews:** Rerun isolated Standards and Spec reviewers, await both reports, and repeat the review loop.
         - **(b) Skip revalidation:** Proceed directly to the delivery gate.
     - If no findings are selected or all are resolved, proceed to the delivery gate.

---

### Phase 4: Completion & delivery / integration gate

Before completing the execution workflow or applying delivery / integration changes:

1. **Finish execution run:**
   - Determine finish outcome:
     - `succeeded`: All wave validations passed (or resolved) and no unresolved review findings exist.
     - `failed`: Carried deferred validation failures or unaddressed review findings remain after reaching cap limits.
     - `blocked`: An unrecoverable blocker was encountered.
   - Write the structured finish report JSON in the session's `.tmp/ivar-run-report.json`.
   - `ivar feature execute finish --print-schema` prints its shape: `summary`, `tasks` and `verification` are required, and `deviations`, `blockers`, `follow_ups` and `agents` are optional. Put any open findings from capped review rounds in `follow_ups`.
   - Close the active execution run receipt:
     ```bash
     ivar feature execute finish <feature> --report-json <path> --outcome <succeeded|failed|blocked>
     ```
   - Run `ivar feature execute status <feature>` and print the output verbatim to the transcript.
   - An agent following this workflow must never attempt integration or delivery while an execution run remains active.

2. **Branch on mode:**
   - **Goal mode:**
     - **STOP HERE.** Do NOT execute `ivar feature integrate` or `ivar feature deliver`.
     - Hand delivery and integration choice to the human with a concise summary:
       ```text
       Execution run finished with outcome: <outcome>.
       Status: <quoted output of ivar feature execute status <feature>>
       Integrate this subfeature from the parent session: ivar feature integrate <feature>
       To deliver this root feature: /ivar-deliver (or ivar feature deliver <feature> --preview)
       ```
   - **Default mode:**
     - Inspect the feature status from `ivar feature status <feature> --json` and check `is_subfeature`.
     - **Subfeature (`is_subfeature == true`):**
       Do not integrate from this session: a child's session ends at `ivar feature execute finish`.
       Print "Integrate from the parent session: `ivar feature integrate <feature>`" (add `--via pr` if integration via PR is configured / requested).
     - **Root feature (`is_subfeature == false`):**
       - **(a) Prompt delivery choice:**
         Ask the human to choose between:
         - **(1) Draft delivery (Default):** Create or update pull requests in draft mode.
         - **(2) Ready for review:** Create or update pull requests ready for review.
         - **(3) Cancel / Defer:** Exit without making changes.
       - **(b) Deliver through the skill:**
         Load the `ivar-deliver` skill (`/ivar-deliver`) and follow it end to end: its PR metadata and title guidance, the `HALL.md` relation checkpoint between preview and apply, and its fingerprint rules. Preview with `ivar feature deliver <feature> --preview`, show the human the preview and fingerprint `<fp>`, and apply with `ivar feature deliver <feature> --fingerprint <fp>` after the human confirms. For Draft delivery, pass `--draft` to both the preview and the apply.
