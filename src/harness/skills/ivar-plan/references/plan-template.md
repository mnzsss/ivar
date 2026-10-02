# Plan template

Synthesize into the REASONS canvas — the sections `ivar plan create`
scaffolds in `plan.md`:
- **Frontmatter** — `repos:` lists every hall repo this plan edits. `ivar plan approve <feature> plan` refuses names absent from `ivar.json`, and `ivar session connect` promotes the listed repos once the plan is approved. Read-only context repos stay out of the list.
- **Entities** — domain model, delta only
- **Approach** — the chosen design, and what was rejected
- **Structure** — file/module organization
- **Changes** — implementation organized into sequential waves (`### Wave N — <outcome>`)
  with point budget (`**Budget:** 0 / 8 points`, ceiling 8 per wave), prerequisites, a
  task table (`| Task | Points | Blocked by | Outcome |`), and exit criteria.
  When the work splits into parallel subfeatures, Wave 0 lands everything two
  children would both edit (dependency manifests, routes, theme, shared registries)
  on the parent first.
- **Lightweight validation** — per-wave executable commands verifying observable contracts.
- **Verification** — the checks that demonstrate the change is complete.
  Each build or test check must be at least as wide as the readers the
  packets declare: a check narrower than its blast radius reports green
  while a reader outside it breaks. A reader no command can check — prose
  stating a count, a fixture — is verified by reading it.
- **Norms** — coding conventions this feature follows. Every behavioural task is Test-Driven (Red → Green → Refactor).
- **Safeguards** — things to watch out for

Use `ivar graph explore <query>` to check direct and transitive consumers, relation paths, and blast radius when populating task readers, interfaces, and safeguards. Fall back to direct source grep and file reads when graph evidence is absent, empty, or unmodeled.

`plan.md` is frozen once approved: any edit, even ticking a box, sends the
gate back to needs-revision. Progress and deferred failures live in
`ivar feature execute checkpoint`, never in this file.

When Requirements and Analysis exist, reference them near the top of the
canvas (for example `Requirements: ../../requirements.md
(approved).`) rather than repeating their content.

## Skeleton

```markdown
---
repos: [<repo>, ...]
---
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

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/01-<semantic-name>.md` | 1 | — | <outcome> |

#### Lightweight validation

- `<exact command>` — <observable contract checked>

#### Exit criteria

- Lightweight validation passes.
- Executed points ≤ 8.

## Verification

List the checks that demonstrate the change is complete.

## Norms

Conventions this feature follows. Every behavioural task follows Red → Green → Refactor.

## Safeguards

What to watch out for.
```
