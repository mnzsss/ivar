# ADR-0002 — One name, two homes

- **Status:** accepted
- **Date:** 2026-08-30

## Context

A unit of work produces two kinds of writing. **Memory** — why this was
done, what was considered, what was rejected — is worth keeping after the
work ships. **Execution** — requirements, analysis, plan — is the SPDD
artifact set that drives the work and is meaningful mostly while it runs.

Execution has a home: `plans/<name>/`, committed, with approval gates in
`.ivar/features/<name>/planning/approvals.json`. Memory has none. It is
held in the agent's context and lost when the session ends.

The obvious fix — one folder per unit of work, `docs/<name>/` holding both
— collides with a convention already shipped. `feature cleanup` writes
`docs/product/NNN-<slug>.md`, `docs/updates/NNN-<slug>.md`, and
`docs/repo-relations/NNN-<slug>.md`, and validates that its record
resolves inside `docs/updates/` (`src/action/feature/cleanup.rs:129`).
Under one folder, a unit of work named `updates` is indistinguishable from
that topic directory.

## Decisions

### D1 — Split by durability, not by unit

Memory is committed at `docs/<name>/` (`discovery.md`, `research/`).
Execution stays at `plans/<name>/` (`requirements.md`, `analysis.md`,
`plan.md`). Local state stays at `.ivar/features/<name>/`.

One name spans all three. The name is the join key; the directories are
not merged.

This is a cut by *what survives the work*, not by *what belongs to the
work*. A unit of work therefore remains spread across three locations —
accepted knowingly, because the alternative collides with `docs/`'s
existing meaning and because it removes a `plans/ → docs/` migration from
scope entirely.

### D2 — The reserved names are enforced in the type, not at the call site

`product`, `updates`, and `repo-relations` are rejected by
`validate_feature` in `src/domain/name.rs`, so a colliding `FeatureName`
cannot be constructed at all. A `Layout`-level guard would have to be
repeated at every call site and could be forgotten at one.

### D3 — Deleting execution never deletes memory

`feature delete` removes `plans/<name>/` and `.ivar/features/<name>/`. It
never touches `docs/<name>/`. This already holds by construction —
`src/action/feature/delete.rs:241` removes `plan_dir(&name)` and nothing
under `docs/` — and is kept honest by test, not by new code.

The reasoning: a unit of work that was explored and abandoned produces the
cheapest information a team owns, and deleting the feature is exactly when
that information stops being re-derivable.

## Consequences

- `Layout` gains `work_dir`, `discovery_doc`, `research_dir`; `plan_dir`
  is unchanged and keeps its name.
- No on-disk format migration. `plans/` does not move.
- Approval gates are unaffected: they are keyed by name under
  `.ivar/features/<name>/planning/`, never by path.
- A pre-existing feature whose name is not kebab-case, or is one of the
  three reserved names, must be renamed before it can be used again. No
  such name exists in-tree at the time of writing.
- `docs/<name>/` is committed, so memory arrives with `git pull` in a
  fresh clone.
