---
description: Document a delivered feature, capture human approvals, and clean up local feature resources safely.
argument-hint: <feature-name>
---

# Feature Cleanup

`/ivar-feature-cleanup` orchestrates the human-gated end-of-feature cleanup process. It inspects a delivered feature, presents delivery evidence, proposes and updates durable product and update pages under `docs/memory/`, collects human approvals across three gates, writes a durable JSON cleanup record, and performs safe, deterministic local teardown using `ivar feature cleanup`.

## Usage

Preview the cleanup state without side effects:

```bash
ivar feature cleanup <feature> --preview
```

Apply an approved cleanup record:

```bash
ivar feature cleanup <feature> --record docs/memory/update/<NNN>-<feature>.cleanup.json
```

## Prerequisites

- You must be inside a **hall** (`ivar status` succeeds).
- The target feature must exist in `ivar feature list`.
- Feature sessions are supported, but **discovery sessions** are rejected. A
  discovery session is one where `ivar session env` prints `IVAR_SESSION_ID`
  but no `IVAR_FEATURE`.

## Feature Resolution

1. Resolve the feature name from `$ARGUMENTS`.
2. If `$ARGUMENTS` is empty, fall back to `$IVAR_FEATURE`.
3. If neither is set, ask which feature to clean up.
4. Refuse discovery sessions and unknown features conversationally before gathering evidence.

## Process Overview & Approval Gates

The cleanup workflow owns three explicit human decisions, recorded independently. Present **one question per turn** with no bundled consent:

1. **Delivery Gate** — Approve evidence that every promoted repository is delivered.
2. **Documentation Gate** — Approve the exact `docs/memory/*` diff, or explicitly approve `not_required` with a human reason.
3. **Teardown Gate** — Approve the exact local sessions, worktrees, metadata, working documents in the feature directory, and local branches to be removed.

## Workflow Steps

### Step 1: Feature Resolution & Preview Evidence

Run `ivar feature cleanup <feature> --preview` to gather local state and evidence without side effects:
- Check reachability of feature HEAD from effective base in each promoted repo.
- Collect local branch presence, worktree status, active sessions, dirty worktrees, missing clones, and paths to be removed.
- When the feature has subfeatures, the preview covers the whole subtree: each descendant is listed (as a tree) with its integration state, repos, blockers and paths. An `integrated` or `abandoned` descendant is eligible; an `active`, `failed` or `stale` one blocks the whole cleanup. The fingerprint covers every node.
- Recompute the preview SHA-256 fingerprint over canonical JSON.

### Step 2: Delivery Gate

Present delivery evidence for every promoted repository in manifest order:
- Show commit reachability from effective base and PR status (if available).
- Highlight empty features explicitly.
- Ask the human for explicit delivery approval:
  "Do you approve that feature '<feature>' is delivered across all promoted repositories?"
- If unmerged commits or missing repos exist as delivery blockers, explain them clearly and stop.

### Step 3: Documentation Gate

Read the feature plan (`.ivar/features/<feature>/plan.md` or `../../plan.md` from a session), per-repo commits/diffs for promoted repos, and existing pages in `docs/memory/product/`, `docs/memory/update/`, and `docs/repo-relations/`.

Propose the minimum durable documentation:
- **Product documentation** (`docs/memory/product/NNN-<slug>.md`): Update an existing product topic before creating a duplicate. Describe current durable product behavior.
- **Update documentation** (`docs/memory/update/NNN-<slug>.md`): Add an update topic only when the delivery matters to future users or operators.
- **Repository relations**: Delegate changed repository relations to the established `/ivar-relations` contract. Never rewrite relation prose yourself.
- **Not required**: Allow an explicit `not_required` decision with a human reason (e.g. internal refactor with no user-visible change).

Rules for documentation writes:
- Show the exact documentation diff to the human.
- Write **nothing** until the documentation gate is explicitly approved.
- Re-read every target file immediately before writing and stop on concurrent change.
- Numbers are monotonic within each directory (three digits, e.g. `001`, `002`). The next update number is the maximum of `docs/memory/update/` plus one, counting `.cleanup.json` files, which share the sequence. Product numbers are the maximum of `docs/memory/product/` plus one. Never reuse or renumber identifiers.

### Step 4: Record Creation & Re-preview

Write the durable cleanup record to `docs/memory/update/<NNN>-<feature>.cleanup.json`.

After documentation writes, re-run `ivar feature cleanup <feature> --preview` because hall state or Git refs may have changed, and bind the final record to that new fingerprint.

### Step 5: Teardown Gate

Present the exact teardown set:
- Local sessions to be stopped/removed.
- Worktrees, metadata, and plan paths to be removed.
- Local feature branches to be deleted per promoted repo.
- For a feature with subfeatures: every descendant's sessions, worktrees, local branches and feature directory, torn down leaves first. One record approves the whole subtree.

State the irreversible safeguard clearly:
> **Warning**: Local teardown is irreversible. Remote branches are evidence and will remain untouched.

Ask for explicit human teardown approval after presenting this set and warning.

### Step 6: Apply Teardown

Update `approvals.teardown.approved` to `true` in the cleanup record and execute:

```bash
ivar feature cleanup <feature> --record docs/memory/update/<NNN>-<feature>.cleanup.json
```

On partial failure:
- Preserve the cleanup record with its partial outcome log.
- Explain the failure and instructions to retry.
- Never turn a failed apply into an assertion of success.
- A subtree apply stops at the first node it cannot fully remove; that node and every ancestor keep their records. Re-run the preview — removed descendants drop out — and bind the record to the new fingerprint.

## Cleanup Record Schema

The durable cleanup record is stored at `docs/memory/update/<NNN>-<feature>.cleanup.json`.

### JSON Schema

```json
{
  "schema_version": 1,
  "feature": "feature-cleanup",
  "branch": "feature-cleanup",
  "fingerprint": "sha256:…",
  "approvals": {
    "delivery": {
      "approved": true,
      "at": "2026-08-28T12:00:00Z"
    },
    "documentation": {
      "decision": "written",
      "paths": [
        "docs/memory/product/003-feature-cleanup.md",
        "docs/memory/update/007-feature-cleanup.md"
      ],
      "reason": null,
      "at": "2026-08-28T12:05:00Z"
    },
    "teardown": {
      "approved": true,
      "at": "2026-08-28T12:10:00Z"
    }
  },
  "outcome": null
}
```

### Field Rules

- `schema_version` is `1`. An unknown version is refused, never migrated.
- `feature` must equal the feature being cleaned; `branch` must equal that feature's recorded branch.
- `fingerprint` references the preview by hash. The preview is **not** inlined, so the record cannot drift from the value the CLI recomputes.
- `approvals.documentation.decision` is `"written"` or `"not_required"`.
  - `"written"` requires a non-empty `paths` array and a null `reason`.
  - `"not_required"` requires a non-empty `reason` string and an empty `paths` array.
- Every `paths` entry is hall-relative and must resolve inside `docs/`.
- `at` is an ISO-8601 UTC timestamp.
- `approvals.delivery.approved` and `approvals.teardown.approved` must both be `true`. `false` is a valid record and a refusal to apply, never a bypass.
- `outcome` is `null` until apply completes; apply writes the final per-repository and teardown results into it. A record whose `outcome` is already populated describes a finished run and is refused as an authorization for a new one.
- `outcome.descendants` lists each removed subfeature in teardown order (leaves first). It is omitted for a feature with no subfeatures, so such records are unchanged.
