---
name: ivar-execute
description: Execute an approved feature plan wave by wave with subagent isolation, lightweight validation, dual-axis review barrier, and gated delivery.
argument-hint: [plan-path]
---

# ivar-execute

`ivar-execute` executes an approved feature plan wave by wave. It acts strictly as a coordinator: it dispatches implementation to subagents, runs lightweight validation at wave checkpoints, records progress and deferred validation failures in the run receipt with `ivar feature execute checkpoint`, barriers at post-wave dual reviews (Standards review and Spec review), and facilitates human-gated delivery. Never edit `plan.md` during a run: any edit moves its fingerprint and diverges the run.

## Interactive choice convention

For every question with selectable choices:
- If the harness provides an interactive `Ask` tool (or prompt choice mechanism), use `Ask`.
- If `Ask` is unavailable, present the question textually with equivalent lettered options (`a`, `b`, `c`, ...).
- Questions requiring open-ended input remain plain textual prompts. Never bundle multiple questions into a single prompt.

## Execution workflow

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Validate Feature State & Plan                           │
│    ├─ Verify plan approval via ivar feature status --json   │
│    ├─ Verify wave lightweight validation contracts         │
│    └─ Initialize/resume execution run receipt:             │
│         ivar feature execute start <feature>               │
│         (pass --plan <plan-path> if not default)           │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 2. Wave Execution Loop (Wave 1 .. Wave N)                  │
│    ├─ Dispatch wave tasks exclusively to subagents          │
│    ├─ Coordinator NEVER edits code directly                 │
│    ├─ Run wave lightweight validation commands              │
│    │    ├─ Pass: prompt human for wave approval             │
│    │    └─ Fail: ask [Fix now] vs [Defer failure]           │
│    │         ├─ Fix now: dispatch subagent, rerun check     │
│    │         └─ Defer: note for the wave checkpoint         │
│    └─ On human approval: ivar feature execute checkpoint    │
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
│         ├─ Ask human which findings to fix                  │
│         ├─ Dispatch selected fixes to subagent              │
│         └─ Ask human: [Rerun both reviews] vs [Skip]        │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 4. Completion & Delivery / Integration Gate                 │
│    ├─ Close active execution run:                           │
│    │    ivar feature execute finish <feature>               │
│    │      --report-json <path> --outcome <outcome>          │
│    ├─ Inspect is_subfeature via ivar feature status --json  │
│    ├─ Subfeature (is_subfeature == true):                   │
│    │    ivar feature integrate <feature> [--via pr]         │
│    └─ Root feature (is_subfeature == false):                │
│         ├─ Default choice: Draft delivery                   │
│         └─ Follow the ivar-deliver skill: preview, then     │
│            apply with the confirmed fingerprint             │
└─────────────────────────────────────────────────────────────┘
```

---

### Phase 1: Preparation & plan validation

1. **Locate feature session & plan:**
   - Resolve `$IVAR_FEATURE` and `$IVAR_SESSION_PATH`.
   - Resolve target plan from `$ARGUMENTS` or default to `../../plan.md` relative to `$IVAR_SESSION_PATH`.
2. **Verify plan approval:**
   - Run `ivar feature status <feature> --json` to verify the plan artifact is approved. If not approved, stop and instruct the user to approve the plan first.
3. **Verify wave validation contracts:**
   - Inspect every wave defined in `plan.md`.
   - **Lightweight validation requirement:** Every wave MUST explicitly define lightweight validation commands (such as scoped unit tests, type checks, or targeted linter commands).
   - If any wave lacks explicit lightweight validation commands, stop execution immediately and require the plan to be updated and re-approved before proceeding.
4. **Initialize or resume run receipt:**
   - Run `ivar feature execute start <feature>` (or `ivar feature execute start <feature> --plan <plan-path>` if using a non-default plan location).
   - This initializes or resumes the active execution run receipt for the feature, capturing the plan fingerprint and state tracking before execution tasks begin.
---

### Phase 2: Wave execution loop

Process waves strictly in sequential plan order (Wave 1, then Wave 2, up to Wave N). Never start Wave `K+1` before Wave `K` reaches human approval.

For each wave:

1. **Subagent dispatch:**
   - Dispatch the wave's task packets exclusively to implementation subagent(s).
   - Build every dispatch prompt from `references/subagent.md`, filling each field with absolute paths. A dispatch missing the working directory, worktree root, or artifact paths is incomplete.
   - **Coordinator isolation invariant:** The coordinator NEVER writes or edits feature code directly. All code edits, refactors, and test additions MUST be performed by subagents.
2. **Execute lightweight validation:**
   - Run the explicit lightweight validation commands `plan.md` declares for this wave.
   - When validating changed files across touched repos, query `ivar graph affected <files...>` to discover affected tests and runnable commands. Graph recommendations are advisory: use them to focus verification, and fall back directly to declared wave commands, project test runners, or targeted file execution when graph queries return empty results, fail, or lack runner configuration.
   - **Case A: Validation Passes**
     - Present the wave result, completed tasks, and validation output.
     - Prompt the human for explicit approval to complete the wave.
   - **Case B: Validation Fails**
     - Report the exact validation failure and command output to the human.
     - Ask the human to choose between:
       - **(a) Fix now:** Dispatch the fix to an implementation subagent, rerun the lightweight validation commands, and return to the checkpoint.
       - **(b) Defer failure:** Keep the failure for this wave's checkpoint summary under "Deferred validation failures". With human approval, allow advancing while carrying the failure into the final review and correction cycle.
4. **Record wave checkpoint:**
   - Upon receiving explicit human approval, append the wave to the run receipt:
     ```bash
     ivar feature execute checkpoint <feature> --wave <n> --summary "<completed tasks; exit criteria met; Deferred validation failures: <list or none>>"
     ```
   - Never edit `plan.md` during a run — not task checkboxes, not wave notes, not deferred failures. Task packets and `plan.md` are read-only inputs.
   - If the plan itself must change, stop and ask the human; re-approval followed by `ivar feature execute accept-revision` is their decision.

---

### Phase 3: Post-wave review barrier & correction cycle

Once all planned waves are approved, execute the final dual-axis review:

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
   - Close the active execution run receipt before proceeding to integration or delivery:
     ```bash
     ivar feature execute finish <feature> --report-json <path> --outcome <succeeded|failed|blocked>
     ```
   - Write the report JSON in the session's `.tmp/` first. `ivar feature execute finish --print-schema` prints its shape: `summary`, `tasks` and `verification` are required, and `deviations`, `blockers`, `follow_ups` and `agents` are optional.
   - An agent following this workflow must never attempt integration or delivery while an execution run remains active.

2. **Branch on feature hierarchy:**
   - Inspect the feature status from `ivar feature status <feature> --json`.
   - Check the `is_subfeature` field:

3. **Branch A: Subfeature (`is_subfeature == true`):**
   - Subfeatures integrate into their parent feature rather than delivering directly to remotes.
   - Run:
     ```bash
     ivar feature integrate <feature>
     ```
     (or `ivar feature integrate <feature> --via pr` if integration via PR is configured / requested).

4. **Branch B: Root feature (`is_subfeature == false`):**
   - Root features deliver to remote repositories through the gated delivery workflow:
     - **(a) Prompt delivery choice:**
       - Ask the human to choose the delivery mode:
         - **(1) Draft delivery (Default):** Create or update pull requests in draft mode.
         - **(2) Ready for review:** Create or update pull requests ready for review.
         - **(3) Cancel / Defer:** Exit without making changes.
     - **(b) Deliver through the skill:**
       - Load the `ivar-deliver` skill (`/ivar-deliver`) and follow it end to end: its PR metadata and title guidance, the `HALL.md` relation checkpoint between preview and apply, and its fingerprint rules.
       - Preview with `ivar feature deliver <feature> --preview`, show the human the preview and fingerprint `<fp>`, and apply with `ivar feature deliver <feature> --fingerprint <fp>` after the human confirms.
       - For Draft delivery, pass `--draft` to both the preview and the apply.
