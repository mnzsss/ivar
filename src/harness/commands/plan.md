---
description: Conduct the SPDD planning process — Requirements, Analysis, Plan, and approval gates.
argument-hint: <feature-name>
---

# Plan

`/ivar-plan` runs the SPDD planning process for a feature. It has three planning
phases, each followed by a human approval gate.

The feature to plan is `$ARGUMENTS`. When that is empty, fall back to
`$IVAR_FEATURE`; with neither, ask which feature to plan. Every `<feature>`
below is that resolved name.

## Prerequisites

- You must be inside a **Feature Session** (`IVAR_FEATURE` must be set).
- The feature must exist (`ivar feature list`).
- Start a new SPDD flow with `ivar plan create <feature>` to scaffold the
  planning artifacts. Name a subset — `ivar plan create <feature> plan` — to
  scaffold only that one; see "The short path" below for when that is
  appropriate.

## Process Overview

The planning lifecycle has three artifacts and three approval gates:

```
Requirements → [approve requirements] → Analysis → [approve analysis] → Plan → [approve plan] → Execution
```

Each artifact lives committed under `<hall>/plans/<feature>/`. Inside a feature
session the same directory is projected into the view dir, so the artifacts are
reachable at `plans/<feature>/` relative to `$IVAR_SESSION_PATH` — edits there
land in the hall's committed directory. Once an artifact is approved, changing
it cascades invalidation to downstream artifacts.

Full SPDD — all three artifacts, all three approvals — is the default and the
right choice whenever there is a real decision to review. An artifact that is
never written is not a gate, though: `ivar plan approve` only requires the
upstream artifacts that actually exist on disk. See "The short path" below
before deciding to skip Requirements or Analysis.

## The short path

For a change with no real design risk — a typo fix, a one-line config change,
a version bump — writing Requirements and Analysis is pure overhead. Skip
straight to Plan:

1. `ivar plan create <feature> plan` scaffolds only `plan.md`.
2. Write it, following Phase 3 below.
3. `ivar plan approve <feature> plan` succeeds on its own: with
   `requirements.md` and `analysis.md` absent, there is no upstream gate left
   to block it.

This only holds while those two files stay unwritten. The moment either is
written, it blocks `plan approve` exactly as it would in full SPDD, until it
is approved too — the escape is "never written," never "written and ignored."
`ivar plan create <feature> requirements analysis` is the upgrade path back to
full SPDD from here: it writes only the artifacts you are missing.

Writing either file back is what ends the short path, and it ends it
immediately: an approved Plan gate whose upstream artifact has just appeared
unapproved drops to `needs-revision`, and `ivar feature deliver` refuses until
you approve the new artifact and re-approve the plan. That is the same rule
`plan approve` enforces, applied to an approval already granted — the tool will
not report a gate approved that it would now decline to grant.

Do not use the short path for anything with real design risk: a new module
boundary, a schema or API change, a new external dependency, anything that
touches more than one repo, anything you would want a teammate to weigh in on
before it is built. That work earns the full three artifacts. Nothing
technical enforces this beyond judgement — `plan create` writing all three by
default is the only guard, and the rest is on you.

## Phase 1: Requirements

1. Research the feature and its context (repos, existing code, user needs).

2. Write the Requirements artifact to `plans/<feature>/requirements.md`. Include:
   - Functional requirements (R-* IDs: R-LOGIN, R-AUTH, etc.)
   - Non-functional requirements (performance, security)
   - Constraints

3. **Pause for human approval.** Show the requirements to the user. Only
   proceed after they approve.

4. Call `ivar plan approve <feature> requirements`.

## Phase 2: Analysis

1. Read `HALL.md` before analyzing. Select the relations involving potentially
affected Repos, follow only the linked topics, and record the relevant context
in `analysis.md`. Offer `/ivar-relations` only when cited code evidence
contradicts, extends, or obsoletes the prose — and deferring that review never
blocks approval of this artifact. This checkpoint never edits `HALL.md`;
`/ivar-relations` is the only writer of the relation region.

2. With approved Requirements as context, analyze the codebase to determine:
   - Affected modules (repo + path + impact level)
   - Trade-offs between approaches
   - Risks and mitigations
   - Recommendations

3. Write the Analysis artifact to `plans/<feature>/analysis.md`.

4. **Pause for human approval.** Show the analysis to the user. Only proceed
after they approve.

5. Call `ivar plan approve <feature> analysis`.

## Phase 3: Plan

1. Synthesize into the REASONS canvas — the sections `ivar plan create`
   scaffolds in `plan.md`:
   - **Entities** — domain model, delta only
   - **Approach** — the chosen design, and what was rejected
   - **Structure** — file/module organization
   - **Changes** — implementation organized into sequential waves (`### Wave N — <outcome>`)
     with point budget (`**Budget:** 0 / 8 points`, ceiling 8 per wave), prerequisites, a
     task table (`| Task | Points | Blocked by | Outcome | Done |` — `[x]` when a task is
     complete, `[ ]` while pending), checkboxed exit criteria (`- [ ]`, flipped to `- [x]`
     as each is met), and a wave-complete marker (`### Wave N — <outcome> ✅` once every
     exit criterion is met).
   - **Verification** — the checks that demonstrate the change is complete
   - **Norms** — coding conventions this feature follows. Every behavioural task is Test-Driven (Red → Green → Refactor).
   - **Safeguards** — things to watch out for

   When Requirements and Analysis exist, reference them near the top of the
   canvas (for example `Requirements: plans/<feature>/requirements.md
   (approved).`) rather than repeating their content.

2. Write the Plan artifact to `plans/<feature>/plan.md`.

3. Generate task packets into `plans/<feature>/tasks/NN-<semantic-task-name>.md` for
   every task in `plan.md`. Name files with a two-digit order prefix (e.g.
   `01-pin-scaffold.md`). Each task packet must follow this structure:

   ```
   ### Task N: [Component Name]
   **Files:**
   - Create: `exact/path.rs`
   - Modify: `exact/path.rs:123-145`
   - Test: `tests/exact/path.rs`
   **Readers:**
   - For each symbol this packet writes, paste the output of
     `grep -rn '<symbol>' tests/ src/`
   - Name the constraint each reader imposes (an assertion, a caller, a config
     consumer)
   - When the grep is empty, write `no readers outside the declared files`
   **Interfaces:**
   - Consumes: [exact signatures from earlier tasks]
   - Produces: [exact function names + types later tasks rely on]
   - [ ] **Step 1: Write the failing test**
   - [ ] **Step 2: Run test to verify it fails**  Run: `...`  Expected: FAIL ...
   - [ ] **Step 3: Write minimal implementation**
   - [ ] **Step 4: Run test to verify it passes**  Run: `...`  Expected: PASS
   - [ ] **Step 5: Commit**
   ```

   A packet whose Readers section is absent or unrun is incomplete in the same
   way a missing test step is.

   No placeholders anywhere: no `TBD`/`TODO`, no "implement later", no "add error handling",
   no "similar to Task N", no step that says what to do without showing how, no reference to a
   symbol defined nowhere. Steps 1–2 are Red (write failing test, run to verify FAIL),
   Steps 3–4 are Green (minimal code, run to verify PASS). Refactoring is permitted between
   Step 4 and Step 5.

4. Dispatch a plan-document reviewer subagent to review `plan.md` and `plans/<feature>/tasks/`.
   **Run it on the smallest capable model this harness offers, never the
   coordinator's.** The pass reads finished documents against a checklist, so
   it does not need the model that wrote them. On Claude Code, set the subagent
   tool's `model` to `haiku`, and to `sonnet` only when `haiku` cannot hold the
   plan. On OpenCode, dispatch through an agent whose configured model is that
   provider's small tier; when only the default agent exists, use it and say so
   in the report. Report the model you ran, so nobody has to guess whether the
   review fell back to the coordinator's.

   The subagent evaluates the plan against `requirements.md` (the spec) across four categories:

   | Category | What to Look For |
   |---|---|
   | Completeness | TODOs, placeholders, incomplete tasks, missing steps |
   | Spec Alignment | Plan covers `requirements.md` (the spec), no major scope creep |
   | Task Decomposition | Tasks have clear boundaries, steps are actionable |
   | Buildability | Could an engineer follow this plan without getting stuck? |
   | Blast Radius | Every packet has a Readers section with real grep output; each reader's constraint is named |

   Reviewer output format:

   ```
   ## Plan Review
   **Status:** Approved | Issues Found
   **Issues (if any):**
   - [Task X, Step Y]: [specific issue] - [why it matters]
   **Recommendations (advisory, do not block approval):**
   - [suggestions]
   ```

   Calibration: approve unless there are serious gaps; minor wording and "nice to have" suggestions do not block approval. When the reviewer raises issues, update the plan/tasks and re-review, at most twice. If the second re-review still reports Issues Found, list what remains and hand it to the human gate in step 5.

5. **Pause for human approval.** Show the plan, task packets, and plan review status to the user. Only proceed after they approve.

6. Call `ivar plan approve <feature> plan`.

## Execution

After the Plan gate is approved, offer execution — do not start it:

> The plan is approved. Run `/ivar-execute plans/<feature>/plan.md` to
> execute it.

**Never run `/ivar-execute` automatically.** Approving a plan and executing it
are two decisions, and the human makes both. `/ivar-discovery` states the same
rule for its own phase transition: it offers `/ivar-plan` and never runs it.

That workflow executes the plan wave by wave and marks each wave complete in
`plan.md` at each wave checkpoint.
