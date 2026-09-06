# ADR-0005 — Local working documents

- **Status:** accepted
- **Date:** 2026-09-05

## Context

ADR-0002 separated a unit of work into committed memory (`docs/<name>/discovery.md`), committed execution artifacts (`plans/<name>/requirements.md`, `analysis.md`, `plan.md`), and local execution state (`.ivar/features/<name>/`).

That split broke three things:
1. **The guard blocked the agent from its own plan.** The view dir projected symlinks into committed `plans/<feature>/` and `docs/<feature>/`. The guard canonicalises every path and admits only the view dir and the promoted worktrees, so a write through those symlinks landed outside the writable set and the guard denied it.
2. **Half-finished thinking entered git history.** Exploratory plans and discovery drafts became commits in the hall, where they outlived their usefulness.
3. **Three feature names were unusable.** `docs/<name>/` could collide with durable topic documentation, so `validate_feature` reserved `product`, `updates` and `repo-relations`.

## Decisions

### D1 — Supersede ADR-0002 D1 (Working documents move to `.ivar/features/<name>/`)

All SPDD planning artifacts (`requirements.md`, `analysis.md`, `plan.md`, task packets `tasks/NN-*.md`) and converted discovery documents (`discovery.md`) move out of committed `<hall>/plans/<name>/` and `<hall>/docs/<name>/` and into gitignored `.ivar/features/<name>/`.

`<hall>/docs/` holds durable, delivered topic documentation only (`product/`, `updates/`, `repo-relations/`). Ivar never creates a per-feature directory under `<hall>/docs/`.

### D2 — Discovery writes to session view dir and lifts on conversion

`ivar discovery create` and `ivar discovery amend` write `discovery.md` into the active discovery session's view dir (`.ivar/sessions/<id>/discovery.md`).

When `ivar session convert` binds the discovery session to a feature, the first step of the conversion state machine (`Step::LiftDiscovery`) moves `discovery.md` from the session view dir into `.ivar/features/<name>/discovery.md`. If a discovery session is abandoned without conversion, its discovery doc dies with the session.

### D3 — Supersede ADR-0002 D3 (`feature delete` tears down all working documents)

`feature delete` removes `.ivar/features/<name>/`, which contains the working documents, planning approvals, Run Receipts, and sessions. Deleting a feature destroys its undistilled discovery doc and unapproved plans.

The comment previously in `src/action/feature/delete.rs:242-247` stating:
> Only execution is removed. `docs/<name>/` — the discovery doc and its
> research — is deliberately untouched (ADR-0002 D10): a feature that
> was tried and dropped leaves behind the cheapest information a team
> owns, and deleting the feature is precisely when that information
> stops being re-derivable.

no longer holds, so the comment goes with it. To keep a learning, distil it into `docs/product/` or `docs/updates/` through `/ivar-feature-cleanup` before deleting.

### D4 — Writable set widening and accepted gate exposure

The session guard's `WritableSet::from_session` includes the entire feature directory (`.ivar/features/<name>/`). An agent inside the session can now create and edit its own SPDD artifacts and task packets.

That also exposes `planning/approvals.json` and sibling session view dirs to those agents. We accept the exposure rather than invent subdirectories or enforce a gate in code that the filesystem does not enforce (per C-GATE-EXPOSURE).

### D5 — View-dir projections removed

The session view dir no longer materializes symlinks for `plans/<feature>` or `work` pointing into `<hall>/plans/` or `<hall>/docs/`. From within a session view dir (`.ivar/features/<name>/sessions/<id>/`), working documents are accessed directly at `../../` relative to `$IVAR_SESSION_PATH`.

### D6 — Retirement of reserved feature names

No per-feature directory lives under `<hall>/docs/` any more, so `src/domain/name.rs` drops the reservation. `product`, `updates` and `repo-relations` are valid feature names.

## Consequences

- `Layout::plan_dir` resolves to `<hall>/.ivar/features/<name>/`.
- `Layout::discovery_doc` resolves to `<hall>/.ivar/features/<name>/discovery.md`.
- `plans_root`, `work_docs_root`, `work_dir`, and `research_dir` are deleted from `Layout`.
- `docs/` contains only flat topic directories (`product/`, `updates/`, `repo-relations/`).
- Working documents are gitignored, so `git pull` does not carry them between machines.
- `approvals.json` fingerprints continue to validate content hashes without path dependency.
- `doctor` reports legacy working documents left in `<hall>/plans/` or `<hall>/docs/<name>/`.
