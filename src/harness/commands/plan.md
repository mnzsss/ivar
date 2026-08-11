---
description: Conduct the SPDD planning process — Requirements, Analysis, Plan, and approval gates.
argument-hint: <feature-name>
---

# Plan

`/ivar-plan` runs the full SPDD planning process for a feature. This workflow
conducts three planning phases with human approval gates between each.

## Prerequisites

- You must be inside a **Feature Session** (`IVAR_FEATURE` must be set).
- The feature must exist (`ivar feature list`).
- Start a new SPDD flow with `ivar plan create <feature>` to scaffold the
  planning artifacts.

## Process Overview

The SPDD planning lifecycle has three artifacts with four approval gates:

```
Requirements → [approve requirements] → Analysis → [approve analysis] → Plan → [approve plan] → Graph → [approve graph] → Execution
```

Each artifact lives committed under `<hall>/plans/<feature>/`. Inside a
feature session the same directory is projected into the view dir, so the
artifacts are reachable at `plans/<feature>/` relative to
`$IVAR_SESSION_PATH` — edits there land in the hall's committed directory.
Once an artifact is approved, changing it cascades invalidation to downstream
artifacts.

## Phase 1: Requirements

1. Research the feature and its context (repos, existing code, user needs).

2. Write the Requirements artifact to `plans/<feature>/requirements.md`. Include:
   - Functional requirements (R-* IDs: R-LOGIN, R-AUTH, etc.)
   - Non-functional requirements (performance, security)
   - Constraints

3. Call `ivar plan approve <feature> requirements` only after the user has
   reviewed the artifact.

4. **Pause for human approval.** Show the requirements to the user. Only
   proceed after they approve.

## Phase 2: Analysis

1. With approved Requirements as context, analyze the codebase to determine:
   - Affected modules (repo + path + impact level)
   - Trade-offs between approaches
   - Risks and mitigations
   - Recommendations

2. Write the Analysis artifact to `plans/<feature>/analysis.md`.

3. **Pause for human approval.** Show the analysis to the user. Only proceed
   after they approve.

4. Call `ivar plan approve <feature> analysis`.

## Phase 3: Plan

1. Synthesize the Requirements and Analysis into a structured plan. Include:
   - **Requirements** section referencing the artifact
   - **Entities** — domain model (delta only)
   - **Approach** — the chosen design approach
   - **Structure** — file/module organization
   - **Operations** — concrete, testable steps with OP-* IDs:
     - Each operation has: id, title, description, dependsOn, touches, tests,
       doneWhen
     - Operation IDs follow the format `OP-<SLUG>` (e.g. `OP-API-CONTRACT`)
     - Touch sets are file paths identifying what files are affected
   - **Norms** — coding conventions to follow
   - **Safeguards** — things to watch out for

2. Write the Plan artifact to `plans/<feature>/plan.md`.

3. **Pause for human approval.** Show the plan to the user. Only proceed after
   they approve.

4. Call `ivar plan approve <feature> plan`.

## Phase 4: Execution Graph

After the plan is approved, the execution graph must be approved separately:

1. Call `ivar feature execute prepare <feature> --graph-json <path>`.
2. When the board awaits approval, show the generated graph to the user.
3. After approval, call `ivar feature execute approve <feature>` — this crosses
   the execution-graph gate.

## Checking Status

At any point, check approval gate status:
`ivar plan status plans/<feature>/plan.md`

## Important

- **Never hand-edit** approvals under `.ivar/features/<feature>/planning/`.
  Always use the CLI commands.
- Changing an upstream artifact (Requirements → Analysis → Plan) automatically
  marks downstream gates as `needs_revision`.
- Behavior-changing plan edits (Operations or Approach changes) require
  re-approval of affected gates.
- **Replan mode**: If execution is in-flight and the Plan needs structural
  changes, revise the plan and fold it into the board with
  `ivar feature execute replan <feature> --plan <plan-path>`. Behavior-changing
  revisions pause affected workstreams until each acknowledges via
  `ivar feature execute ack-revision <feature> --workstream <id>`. Execution
  resumes only after all affected workstreams acknowledge.
- **Reconcile mode**: For local code divergence confined to an operation's
  implementation, record the deviation in the execution journal with
  `ivar feature execute reconcile <feature> --workstream <id> --description
  <text>`.
