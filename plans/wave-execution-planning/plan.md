# Plan

Wave-based execution planning: `plan.md` organised into sequential waves with
a point budget, task packets in `plans/<feature>/tasks/`, a plan-document
review by subagent before approval, and a coordinator that delegates one task
per subagent and stops for human approval between waves. Every behavioural
task is Test-Driven — Red → Green → Refactor.

Task packet content and the plan review are adapted from the Superpowers
`writing-plans` skill and its reviewer prompt:

- https://github.com/obra/superpowers/tree/main/skills/writing-plans
- https://github.com/obra/superpowers/blob/main/skills/writing-plans/plan-document-reviewer-prompt.md

## Entities

**Wave** — a sequential slice of the plan that delivers a verifiable result
and ends at a checkpoint. Carries a point budget (ceiling 8) and requires
human approval before the next wave starts. Waves are ordered; there is no
parallelism and no dependency graph between waves.

**Task packet** — one Markdown file per task under
`plans/<feature>/tasks/`, named `NN-<semantic-task-name>.md` — e.g.
`03-plan-generates-task-packets.md`: a two-digit order prefix plus a kebab-case
name describing the outcome. A packet follows the writing-plans *Task
Structure*: `Files` (create/modify/test, exact paths), `Interfaces`
(`Consumes`/`Produces`, exact signatures), checkbox steps that each carry real
content plus `Run:` and `Expected:`, and a final `Commit` step. It is the unit
of delegation: one subagent per packet. A packet does **not** record which wave
it belongs to — wave membership, ordering, and budget live only in `plan.md`'s
wave tables, so a task can move between waves without renaming or editing its
file.

**Point** — a coarse measure of complexity/context (1–5 per task), not a token
estimate. Summed per wave against the ceiling.

**Wave checkpoint** — the approval gate between waves. The coordinator stops,
summarises, and asks the human to approve before delegating the next wave.

**Plan review** — a subagent dispatched after the plan document and its task
packets are written, following the `plan-document-reviewer-prompt.md` template.
It checks four categories — Completeness, Spec Alignment, Task Decomposition,
Buildability — and returns `Status: Approved | Issues Found` with concrete
issues and advisory recommendations. Calibration: approve unless there are
serious gaps; minor wording and "nice to have" do not block. In Ivar the
"spec" the reviewer reads is `requirements.md`.

Delta against `docs/glossary.md`: none. These are workflow artifacts, not new
domain state — they do not extend `features/<feature>/planning/approvals.json`
and introduce no fourth approval gate.

## Approach

Today `ivar plan create` scaffolds `plan.md` with a REASONS canvas whose
`## Changes` section only asks for "reviewable steps". `/ivar-execute` then
tells the coordinator to "decompose the approved plan" into subagents with no
budget discipline, no per-task artifact, and no review of the plan itself. A
large plan risks blowing a single agent's context/orchestration budget, and
nothing catches a plan with placeholders or vague tasks before it runs.

Adopt the Superpowers `writing-plans` discipline, adapted to Ivar's SPDD flow:
the plan's `## Changes` becomes waves with a point budget; each task becomes a
self-contained packet written to the writing-plans *Task Structure*; and the
plan is reviewed by a subagent before approval rather than trusted blindly.
The Rust code changes in exactly one place — the `PLAN_TEMPLATE` constant —
because task generation and plan review are coordinator work described in the
command instructions, not new persisted state. The approval gates stay three;
the wave checkpoint and the plan review are workflow instructions, not new
`Gate` values.

Rejected: **a Rust-backed task validator.** The writing-plans reviewer is a
subagent with calibrated judgement ("approve unless serious gaps"), not a
mechanical lint. Encoding the No-Placeholders scan and the Spec-Alignment
check in Rust would turn a review into a grep, and would grow the approval
model for no behavioural gain. The review stays a subagent dispatch in the
command instructions, reading the same reviewer prompt an implementer would.

Rejected: **encoding a local execution graph in the plan.** The plan records
waves and tasks for a provider-native coordinator to read; it does not become a
machine-readable DAG. Waves are strictly sequential, so order is text, not a
graph.

Rejected: **one subagent per wave.** Delegating eight points to one subagent
reproduces the context-blowout this feature exists to prevent. One packet per
subagent keeps each context minimal; the coordinator sequences them.

## Structure

```
src/action/plan/create.rs            PLAN_TEMPLATE (the only Rust change)
src/harness/commands/plan.md         /ivar-plan instructions (task packets + review)
src/harness/commands/execute.md      /ivar-execute instructions (waves + delegation)
tests/unit/action/plan/create.rs     scaffold-content assertion (colocated via #[path])
tests/unit/harness/commands.rs       command frontmatter/$ARGUMENTS guards (unchanged, must pass)
docs/guides/planning-and-execution.md  process prose (align)
```

The writing-plans *Task Structure* and the reviewer prompt are adapted into the
`/ivar-plan` instructions — not vendored as new files, and not fetched at
runtime. The command ships the shape and the review rules inline, with the two
source URLs recorded as provenance. Adaptations from the originals: plan
location `docs/superpowers/plans/…` becomes `plans/<feature>/tasks/`; the
required sub-skill `subagent-driven-development` becomes Ivar's own
one-subagent-per-packet execution; example code uses Rust/cargo, not Python.

The task packets live next to the plan, versioned with it:

```
plans/<feature>/
├── requirements.md
├── analysis.md
├── plan.md
└── tasks/
    ├── 01-pin-scaffold-wave-structure.md
    ├── 02-rewrite-plan-template.md
    └── 03-plan-generates-task-packets.md
```

`plan.md` maps packets to waves in its tables — a packet can move between waves
without renaming; only the table row changes. The `NN` prefix keeps the
on-disk order stable and read order predictable.

## Changes

The implementation, split into sequential waves. Every behavioural task is
Test-Driven: write the failing test, make it pass, then refactor. Documentation
tasks (command instructions, prose) are test-first only where a test already
guards the bytes — `harness::commands` asserts frontmatter and `$ARGUMENTS`
consumption, so those edits are verified by running the existing suite rather
than by new tests.

### Wave 1 — Scaffold template (Rust contract)

**Budget:** 0 / 8 points
**Prerequisites:** none

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/01-pin-scaffold-wave-structure.md` | 2 | — | A test pins the scaffold's wave structure |
| `tasks/02-rewrite-plan-template.md` | 2 | 01 | `PLAN_TEMPLATE` emits the wave structure |

#### Exit criteria

- [ ] `cargo test action::plan::create` passes.
- [ ] `cargo test` (full unit suite) passes — the existing three-artifact and
      skip/backward-compat scaffold tests still hold.
- [ ] Executed points ≤ 8.
- [ ] Deviations recorded.
- [ ] Human approval requested and granted to start Wave 2.

### Wave 2 — `/ivar-plan` writes task packets and reviews the plan

**Budget:** 0 / 8 points
**Prerequisites:** Wave 1 approved

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/03-plan-generates-task-packets.md` | 3 | — | `/ivar-plan` generates `tasks/*.md` in the writing-plans Task Structure |
| `tasks/04-plan-review-subagent.md` | 2 | 03 | `/ivar-plan` dispatches a plan-document reviewer subagent |
| `tasks/05-command-guards-pass.md` | 1 | 04 | Command guards still pass |

#### Exit criteria

- [ ] `cargo test harness::commands` passes (frontmatter, `$ARGUMENTS` consumption).
- [ ] `cargo test --test shipped_commands` passes.
- [ ] The `/ivar-plan` instructions cover: generating one `tasks/*.md` per plan
      task following the writing-plans *Task Structure*, forbidding placeholders,
      and dispatching the reviewer subagent per
      `plan-document-reviewer-prompt.md` with its four categories, calibration,
      and `Approved | Issues Found` output.
- [ ] Executed points ≤ 8.
- [ ] Deviations recorded.
- [ ] Human approval requested and granted to start Wave 3.

### Wave 3 — `/ivar-execute` waves, delegation, approval

**Budget:** 0 / 8 points
**Prerequisites:** Wave 2 approved

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/06-execute-delegates-per-wave.md` | 5 | — | Execute dispatches one packet per subagent, gates waves |
| `tasks/07-document-waves-in-guide.md` | 2 | 06 | Planning guide documents waves/budget/tasks and matches reality |

#### Exit criteria

- [ ] `cargo test harness::commands` passes — including
      `execute_defines_provider_native_receipt_coordination`, which requires the
      strings `provider-native`, `coordinator`, `child Feature`, `accept-revision`,
      and `--report-json` to survive the edit.
- [ ] `cargo test --test shipped_commands` passes.
- [ ] `docs/guides/planning-and-execution.md` names waves, budget, task packets,
      the plan review, and the checkpoint, and no longer describes `## Changes`
      as bare "reviewable steps".
- [ ] Executed points ≤ 8.
- [ ] Deviations recorded.
- [ ] Final wave — human approval requested and granted to deliver the feature.

## Verification

- `cargo test` — full unit suite (the colocated `#[path]` modules).
- `cargo test --test shipped_commands` — the 15 commands materialise and match
  their recorded frontmatter.
- `cargo clippy --all-targets -- -D warnings`.
- `cargo fmt --check`.

The change is complete when a coordinator following `/ivar-plan` on an approved
wave-structured plan produces one `tasks/*.md` per task in the writing-plans
*Task Structure*, dispatches a plan-document reviewer subagent, and — after
`plan approve` — a coordinator following `/ivar-execute` dispatches one subagent
per packet, refuses the next wave without approval, and records results back
into each packet.

## Norms

- TDD is mandatory for behavioural changes: Red → Green → Refactor, test at a
  public seam, never an implementation-coupled test.
- The writing-plans five steps implement the TDD contract: Steps 1–2 are Red
  (write the failing test, run it to fail), Steps 3–4 are Green (minimal
  implementation, run to pass). A refactor pass is allowed between Step 4 and
  Step 5 (Commit) when the minimal implementation needs cleanup, keeping the
  tests green.
- No placeholders in any task packet: no `TBD`/`TODO`, no "implement later", no
  "add error handling", no "similar to Task N", no step that says what to do
  without showing how, no reference to a symbol defined nowhere.
- The scaffold test asserts the scaffold's *observable content* (the markers a
  coordinator depends on), not that a `const &str` equals itself — the test
  reads the file `create()` actually wrote.
- Point budget: 8 per wave, 1–5 per task, points measure complexity not tokens.
- A task packet is the only thing a subagent is handed; the subagent never edits
  `plan.md`.
- Deviations that change scope, points, dependencies, or design block the wave
  and return to the human.

## Safeguards

- `tests/unit/harness/commands.rs` has two guards that must keep passing: every
  command has `description:` frontmatter, and every command declaring
  `argument-hint` must reference `$ARGUMENTS` in its body. Neither edit may drop
  these.
- `execute_defines_provider_native_receipt_coordination` pins five substrings in
  `execute.md`; keep them verbatim.
- `ivar plan create` must still scaffold exactly the three artifacts; adding the
  wave structure to `PLAN_TEMPLATE` must not change the create/backward-compat
  behaviour (`N-BACKWARD-COMPATIBLE`, skip-existing semantics).
- Do not add a fourth `Gate`. The wave checkpoint and the plan review are
  coordinator instructions, not entries in `approvals.json`.
- The reviewer's calibration is "approve unless serious gaps". Do not let the
  review turn into a style pass or an unbounded rewrite loop.
- Scope creep in a subagent must block the task, not silently spend points.
