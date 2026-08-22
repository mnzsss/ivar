---
description: Conduct the SPDD planning process — Requirements, Analysis, Plan, and approval gates.
argument-hint: <feature-name>
---

# Plan

`/ivar-plan` runs the SPDD planning process for a feature. It has three planning
phases, each followed by a human approval gate.

## Prerequisites

- You must be inside a **Feature Session** (`IVAR_FEATURE` must be set).
- The feature must exist (`ivar feature list`).
- Start a new SPDD flow with `ivar plan create <feature>` to scaffold the
  planning artifacts.

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

1. Synthesize the Requirements and Analysis into the REASONS canvas:
   - **Requirements** — referencing the artifact
   - **Entities** — domain model (delta only)
   - **Approach** — the chosen design approach
   - **Structure** — file/module organization
   - **Operations** — implementable units identified by OP-* IDs
   - **Operation details** — what each operation ID means
   - **Norms** — coding conventions to follow
   - **Safeguards** — things to watch out for

2. Write the Plan artifact to `plans/<feature>/plan.md`. Give `Operations`
and `Operation details` the exact shape required by the planning schema.

3. **Pause for human approval.** Show the plan to the user. Only proceed after
they approve.

4. Call `ivar plan approve <feature> plan`.

## Execution

After the Plan gate is approved, begin `/ivar-execute plans/<feature>/plan.md`.
That workflow creates and maintains the provider-neutral Run Receipt while the
active provider coordinates its native subagents.
