---
name: ivar-execute
description: Execute an approved feature plan wave by wave with subagent isolation, lightweight validation, dual-axis review barrier, and gated delivery.
argument-hint: [plan-path]
---

# ivar-execute

`ivar-execute` executes an approved feature plan wave by wave. It acts strictly as a coordinator: it dispatches implementation to subagents, runs lightweight validation at wave checkpoints, records progress and deferred validation failures in `plan.md`, barriers at post-wave dual reviews (Standards review and Spec review), and facilitates human-gated delivery.

## Interactive choice convention

For every question with selectable choices:
- If the harness provides an interactive `Ask` tool (or prompt choice mechanism), use `Ask`.
- If `Ask` is unavailable, present the question textually with equivalent lettered options (`a`, `b`, `c`, ...).
- Questions requiring open-ended input remain plain textual prompts. Never bundle multiple questions into a single prompt.

## Execution workflow

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Validate Feature State & Plan                           │
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
│    │         └─ Defer: record in plan.md, carry forward     │
│    └─ On human approval: record completed wave in plan.md   │
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
│ 4. Gated Delivery Gate                                      │
│    ├─ Default choice: Draft delivery                        │
│    ├─ Run ivar feature deliver <feature> --preview          │
│    └─ Apply delivery with confirmed fingerprint             │
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

---

### Phase 2: Wave execution loop

Process waves strictly in sequential plan order (Wave 1, then Wave 2, up to Wave N). Never start Wave `K+1` before Wave `K` reaches human approval.

For each wave:

1. **Subagent dispatch:**
   - Dispatch the wave's task packets exclusively to implementation subagent(s).
   - **Coordinator isolation invariant:** The coordinator NEVER writes or edits feature code directly. All code edits, refactors, and test additions MUST be performed by subagents.
2. **Execute lightweight validation:**
   - Run the explicit lightweight validation commands declared for this wave in `plan.md`.
3. **Handle validation results:**
   - **Case A: Validation Passes**
     - Present the wave result, completed tasks, and validation output.
     - Prompt the human for explicit approval to complete the wave.
   - **Case B: Validation Fails**
     - Report the exact validation failure and command output to the human.
     - Ask the human to choose between:
       - **(a) Fix now:** Dispatch the fix to an implementation subagent, rerun the lightweight validation commands, and return to the checkpoint.
       - **(b) Defer failure:** Record the failure under "Deferred validation failures" for this wave in `plan.md`. With human approval, allow advancing while carrying the failure into the final review and correction cycle.
4. **Record wave checkpoint:**
   - Upon receiving explicit human approval, update `plan.md`:
     - Mark completed tasks.
     - Note satisfied exit criteria.
     - Record any Deferred validation failures.
     - Mark the wave complete.
   - Re-approve the plan artifact if required by the feature session state.

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
   - Include any unresolved Deferred validation failures carried forward from previous waves.
4. **Fix selection & revalidation loop:**
   - Ask the human which reported findings (if any) should be addressed.
   - If findings are selected:
     - Dispatch the selected fixes exclusively to an implementation subagent.
     - Ask the human:
       - **(a) Rerun both reviews:** Rerun isolated Standards and Spec reviewers, await both reports, and repeat the review loop.
       - **(b) Skip revalidation:** Proceed directly to the delivery gate.
   - If no findings are selected or all are resolved, proceed to the delivery gate.

---

### Phase 4: Delivery gate

Before applying any delivery changes or pushing branches:

1. **Prompt delivery choice:**
   - Ask the human to choose the delivery mode:
     - **(a) Draft delivery (Default):** Create or update pull requests in draft mode.
     - **(b) Ready for review:** Create or update pull requests ready for review.
     - **(c) Cancel / Defer:** Exit without making changes.
2. **Preview delivery:**
   - Run side-effect-free preview:
     ```bash
     ivar feature deliver <feature> --preview
     ```
   - Present the preview output and content fingerprint `<fp>` to the human.
3. **Apply delivery:**
   - With explicit human confirmation, apply delivery using the reviewed fingerprint:
     ```bash
     ivar feature deliver <feature> --fingerprint <fp>
     ```
   - If Draft delivery was selected, ensure PRs are submitted in draft mode.
