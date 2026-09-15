# Plan template

Synthesize into the REASONS canvas — the sections `ivar plan create`
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
- **Lightweight validation** — per-wave executable commands verifying observable contracts.
- **Deferred validation failures** — documented failures carried forward.
- **Verification** — the checks that demonstrate the change is complete.
  Each build or test check must be at least as wide as the readers the
  packets declare: a check narrower than its blast radius reports green
  while a reader outside it breaks. A reader no command can check — prose
  stating a count, a fixture — is verified by reading it.
- **Norms** — coding conventions this feature follows. Every behavioural task is Test-Driven (Red → Green → Refactor).
- **Safeguards** — things to watch out for

Use `ivar graph explore <query>` to check direct and transitive consumers, relation paths, and blast radius when populating task readers, interfaces, and safeguards. Fall back to direct source grep and file reads when graph evidence is absent, empty, or unmodeled.

When Requirements and Analysis exist, reference them near the top of the
canvas (for example `Requirements: ../../requirements.md
(approved).`) rather than repeating their content.

## Skeleton

```markdown
# Plan

The REASONS canvas: explain the implementation, its constraints, and how it
will be verified. Every behavioural task is Test-Driven — Red → Green → Refactor.

## Entities

Domain model, delta only.

## Approach

The chosen design, and what was rejected.

## Structure

File and module organization.

## Changes

The implementation, split into sequential waves.

### Wave N — <outcome>

**Budget:** 0 / 8 points
**Prerequisites:** none

| Task | Points | Blocked by | Outcome | Done |
| --- | ---: | --- | --- | --- |
| `tasks/01-<semantic-name>.md` | 1 | — | <outcome> | [ ] |

#### Lightweight validation

- `<exact command>` — <observable contract checked>

#### Deferred validation failures

- None.

#### Exit criteria

- [ ] Verification checks pass.
- [ ] Executed points ≤ 8.
- [ ] Deviations recorded.
- [ ] Human approval requested and granted to start Wave N+1.

## Verification

List the checks that demonstrate the change is complete.

## Norms

Conventions this feature follows. Every behavioural task follows Red → Green → Refactor.

## Safeguards

What to watch out for.
```
