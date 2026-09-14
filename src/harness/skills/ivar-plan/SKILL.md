---
name: ivar-plan
description: Conduct the SPDD planning process — Requirements, Analysis, Plan, and approval gates.
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

Each artifact lives under `.ivar/features/<feature>/`. Inside a feature
session the feature directory sits two levels above the view dir, so the artifacts
are reachable at `../../` relative to `$IVAR_SESSION_PATH` (or by relative filename
when working from the feature directory). Once an artifact is approved, changing
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

2. Write the Requirements artifact to `../../requirements.md` (relative to `$IVAR_SESSION_PATH`). Include:
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
   
   Query `ivar graph explore <query>` to inspect repo-qualified relations, provenance, confidence, entry points, and bounded dependency flow. Graph evidence is advisory: if graph queries are stale, empty, unsupported, or unavailable, fall back directly to source search and file reading. Graph evidence supplements source inspection and never creates, approves, or bypasses an approval gate.
3. Write the Analysis artifact to `../../analysis.md` (relative to `$IVAR_SESSION_PATH`).

4. **Pause for human approval.** Show the analysis to the user. Only proceed
after they approve.

5. Call `ivar plan approve <feature> analysis`.

## Phase 3: Plan

1. Synthesize the REASONS canvas following `references/plan-template.md`.

2. Write the Plan artifact to `../../plan.md` (relative to `$IVAR_SESSION_PATH`).

3. Generate task packets into `../../tasks/NN-<semantic-task-name>.md` following `references/task-template.md`.

4. Dispatch a plan-document reviewer subagent to review `../../plan.md` and `../../tasks/`.
   **Run it on the smallest capable model this harness offers, never the
   coordinator's.** The pass reads finished documents against a checklist, so
   it does not need the model that wrote them. On Claude Code, set the subagent
   tool's `model` to `haiku`, and to `sonnet` only when `haiku` cannot hold the
   plan. On OpenCode, dispatch through an agent whose configured model is that
   provider's small tier; when only the default agent exists, use it and say so
   in the report. Report the model you ran, so nobody has to guess whether the
   review fell back to the coordinator's.

   The subagent evaluates the plan against `requirements.md` (the spec) across these categories:

   | Category | What to Look For |
   |---|---|
   | Completeness | TODOs, placeholders, incomplete tasks, missing steps |
   | Spec Alignment | Plan covers `requirements.md` (the spec), no major scope creep |
   | Task Decomposition | Tasks have clear boundaries, steps are actionable |
   | Buildability | Could an engineer follow this plan without getting stuck? |
   | Blast Radius | Every packet has a Readers section holding real `git grep -n` output run without a pathspec; each reader's constraint is named; the Verification checks are at least as wide as those readers, and a reader no command can check is verified by reading it |
   | Literal Code | Step 1 shows the test's source, never a description of it; Step 3 shows the exact call or signature at each point that decides behaviour; every described step carries `**Sketch:**` whose reason states why literal code is inappropriate there — a marker with a generic or absent reason is an issue |

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

> The plan is approved. Run `ivar-execute ../../plan.md` to
> execute it.

**Never run `ivar-execute` automatically.** Approving a plan and executing it
are two decisions, and the human makes both. `/ivar-discovery` states the same
rule for its own phase transition: it offers `/ivar-plan` and never runs it.

That workflow executes the plan wave by wave and marks each wave complete in
`plan.md` at each wave checkpoint.
