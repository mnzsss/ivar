# ADR-0004 — Local land without a pull request

- **Status:** accepted
- **Date:** 2026-08-31

## Context

`integrate` moves a child feature into its parent and refuses a root
(`src/action/feature/integrate/mod.rs`), with a fix action pointing at
`deliver` — which until now only pushed branches and opened PRs. A root
feature had no local landing path.

The `deliver` verb already held the right semantics for this: roots deliver,
children integrate. Extending `integrate` to accept roots would invert a
decision the code states out loud and strand the fix action pointing the wrong
way. A standalone `feature land` would be a second portal onto delivery, which
the single-portal principle (D1) exists to prevent.

## Decisions

### D3 — Fast-forward or nothing

Land refuses any merge that is not a fast-forward and points at
`ivar feature rebase`.

FF is the only merge that cannot conflict, so a local preflight predicts the
outcome with certainty. The three strategies `integrate` offers (`squash`,
`merge`, `rebase`) would make the preflight a guess: a conflict would surface
mid-batch, breaking the batch in half — precisely what D5 forbids. FF-only is
therefore derived from D5, not a taste preference. Not fast-forwardable blocks
and points at `ivar feature rebase <name>`; land never resolves a conflict.

### D5 — Local preflight, best-effort push

Every promoted repo is validated before any is written; one blocker refuses
the batch. Execution runs in three phases (`src/action/feature/deliver/land.rs`):

0. **Preflight** — for every repo: a rebase in progress, a dirty default
   worktree, a default that does not fast-forward, or an undeclared default
   branch each refuse the batch. Then every repo's ordered verification checks
   run, and a repo whose checks fail refuses the batch too — land writes onto
   default branches, so it verifies at least as strictly as push, which only
   moves a branch a human still reviews.
1. **Validate** — for every repo, re-read the remote default tip and compare
   it against the evidence captured at preview time. Absent evidence or a
   mismatched tip refuses the entire batch. No worktrees are touched.
2. **Merge** — lift write bits on every default worktree, then fast-forward
   each one. If any merge fails, all previously merged repos in the batch
   are rolled back to their original HEAD. Rollback uses `Git::reset_hard`,
   which already existed on main before this feature.
3. **Push** — push each default branch to its remote, best-effort. Push
   failures are warnings, never aborts. The merges stand.

The atomicity promised is therefore **local**: merges are local and gated by
a complete preflight; pushes are independent network operations. There is no
atomic push across N remotes.

**By compensation, not by transaction.** The repos have separate Git
directories, so no transaction spans them. Phase 2 records each default's
original commit and, on a failed merge, resets the already-merged repos back
to it. That compensation is itself fallible: a repo it cannot restore is
reported in `deliver.land_rollback_failed`, carrying both the original merge
failure and every repo left unrestored. This is the one path where the batch
really is half-landed, and it is named rather than hidden — a rollback failure
must not be reported as a clean refusal.

**What the guarantee covers, then.** Every failure land can foresee is caught
in phases 0 and 1, with nothing written: a blocker in the last repo stops the
first from being touched. What remains is the window between the last check
and the write, where only something outside `ivar` can intervene — a
concurrent process, an `index.lock`, a full disk. That class degrades to a
named, reported partial state, never to silence. Re-validating
fast-forwardability, worktree cleanliness, and HEAD movement immediately
before each merge narrows the window further (from the full batch to a
single repo's merge step); it does not close it entirely, as concurrent
edits can still occur during `fast_forward_to` itself.

### Why not `integrate`'s receipt model

Receipts make integration partial and resumable on purpose — right for a tree
walk, wrong for one hop. Two semantics for one operation in one binary is the
duplicate portal D1 exists to prevent.

## Consequences

- A land that merges everywhere but fails to push somewhere leaves that
  default ahead locally; the warning says so and a rerun pushes it.
- A diverged default means a rebase first, always explicit, never resolved
  by land.
- The preview fingerprint includes `DeliveryMode::Land`, so a push-approved
  fingerprint cannot apply as land and vice versa (D11).
- The preview reads the remote (for the unpushed-commits blocker and PR
  action) but the FF check itself is local (bare repo, no fetch). An offline
  preview would be new work and would contradict the existing unpushed-commits
  blocker.

## Git trait additions

One new trait operation was added, in Wave 2: `Git::is_rebase_in_progress`
(`src/git/mod.rs`), checking for `rebase-merge` / `rebase-apply` in the
worktree's git directory. The land preflight uses it to block when a rebase
is in progress on the default branch.

The other git operations land uses — `fast_forward_to`, `reset_hard`,
`head_commit` — already existed on main. No new trait op was needed for the
merge itself.
