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

1. Read `HALL.md` before analyzing. Select the relations involving potentially
   affected Repos, follow only the linked topics, and record the relevant
   context in `analysis.md`. Offer `/ivar-relations` only when cited code
   evidence contradicts, extends, or obsoletes the prose — and deferring that
   review never blocks approval of this artifact. This checkpoint never edits
   `HALL.md`; `/ivar-relations` is the only writer of the relation region.

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
   - **Operations** — which workstream owns which operation ids
   - **Operation details** — what each operation id means
   - **Norms** — coding conventions to follow
   - **Safeguards** — things to watch out for

2. Write the Plan artifact to `plans/<feature>/plan.md`. Give `Operations`
   and `Operation details` the exact shape below — they are parsed, not read.

3. **Pause for human approval.** Show the plan to the user. Only proceed after
   they approve.

4. Call `ivar plan approve <feature> plan`.

### Operations and Operation details are parsed, not read

Every other section is prose for a human. These two are the input `ivar
feature execute tick` parses to build each executor's prompt: the graph names
the `OP-*` ids a workstream owns, and the plan is where the executor is told
what those ids mean. Get their shape wrong and the tick refuses with
`execute.operation_missing_from_plan` — after the graph has already been
approved, with nothing launched.

Write them exactly like this:

<!-- BEGIN PLAN FORMAT EXAMPLE -->
```markdown
## Operations

### checkout-api
- OP-API-CONTRACT
- OP-API-HANDLER
write_contract:
- src/api/checkout.rs
- src/api/checkout_test.rs

## Operation details

**OP-API-CONTRACT** — Define the request and response types for `POST
/checkout`, including the `410` a closed cart answers with.

- `touches`: src/api/checkout.rs
- `tests`: a closed cart answers `410`; an open one answers `200`
- `doneWhen`: the contract compiles and both tests pass

**OP-API-HANDLER** — Implement the handler against that contract, rejecting a
cart the session does not own before any pricing runs.

- `dependsOn`: OP-API-CONTRACT
- `tests`: a foreign cart is rejected before pricing; an owned cart prices once
- `doneWhen`: a foreign cart can no longer reach the pricer
```
<!-- END PLAN FORMAT EXAMPLE -->

The rules behind that shape:

- `### <id>` under `## Operations` is a **workstream id**, and it must match a
  workstream id in the execution graph byte for byte. It is not a phase, a
  cluster, or a title. Grouping operations under `### Fase 1` or `### Cluster
  2 — Report` produces workstreams no graph refers to, and every operation the
  graph claims is then absent from the plan.
- The bullets under it are **operation ids and nothing else** — `- OP-SLUG`,
  one per line, following `OP-<SLUG>` (e.g. `OP-API-CONTRACT`).
- `write_contract:` switches the bullets that follow to the paths that
  workstream may write. `ivar feature execute replan` compares this list to
  decide which workstreams a revision affects, so it is load-bearing.
- Every id needs a `**OP-SLUG**` entry under `## Operation details`. The
  entry's text is handed to the executor verbatim and runs until the next
  declared `**OP-***` marker or the next heading, so a lead paragraph followed
  by a bulleted `dependsOn` / `touches` / `tests` / `doneWhen` block arrives
  whole. A blank line is a paragraph break inside the entry, not its end — the
  heading after the last entry is what ends that one.
- `## Operations` is parsed to the end of the file: every later heading opens
  another workstream named after it. Harmless for `## Norms` and
  `## Safeguards`, but a workstream id must never collide with a section
  heading.

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

## The decision split

When a request arrives while a feature is mid-flight, choose exactly one path:

- **Outside the approved scope + isolatable** → create a child automatically:
  run `ivar feature create <child> --parent <current>`, announce the new
  child, and do not ask permission. The executor reports such requests; the
  coordinator creates.
- **Structural correction to the approved plan** → `ivar feature execute
  replan <feature> --plan <plan-path>`.
- **Implementation-only local divergence** → `ivar feature execute reconcile
  <feature> --workstream <id> --description <text>`.
