---
description: Document a delivered feature, capture human approvals, and clean up local feature resources safely.
argument-hint: <feature-name>
---

# Feature Cleanup

`/ivar-feature-cleanup` orchestrates the human-gated end-of-feature cleanup process. It inspects a delivered feature, presents delivery evidence and the feature's memory review, collects human approvals across two gates, writes a durable JSON cleanup record, and performs safe, deterministic local teardown using `ivar feature cleanup`.

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

The cleanup workflow owns two explicit human decisions, recorded independently. Present **one question per turn** with no bundled consent:

1. **Delivery Gate** — Approve evidence that every promoted repository is delivered, with the feature's memory review.
2. **Teardown Gate** — Approve the exact local sessions, worktrees, metadata, working documents in the feature directory, and local branches to be removed.

Pages are written during the feature and confirmed by `memory_review` before delivery; cleanup writes no page.

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
- Show the feature's memory review from `ivar memory review <feature>`: the pages it wrote (`docs/memory/product/`, `docs/memory/update/`, …), the verdicts and the reason of the last `memory.reviewed` or `memory.review_skipped`.
- When the preview warns `cleanup.memory_not_reviewed`, say the feature was delivered without a memory review. It never blocks cleanup.
- Ask the human for explicit delivery approval:
  "Do you approve that feature '<feature>' is delivered across all promoted repositories?"
- If unmerged commits or missing repos exist as delivery blockers, explain them clearly and stop.

### Step 3: Record Creation & Re-preview

Write the durable cleanup record to `docs/memory/update/<NNN>-<feature>.cleanup.json`. Its number is the maximum of `docs/memory/update/` plus one, counting `.cleanup.json` files, which share the sequence with update pages. Never reuse or renumber identifiers.

Re-run `ivar feature cleanup <feature> --preview` right before writing, because hall state or Git refs may have changed, and bind the record to that fingerprint.

### Step 4: Teardown Gate

Present the exact teardown set:
- Local sessions to be stopped/removed.
- Worktrees, metadata, and plan paths to be removed.
- Local feature branches to be deleted per promoted repo.
- For a feature with subfeatures: every descendant's sessions, worktrees, local branches and feature directory, torn down leaves first. One record approves the whole subtree.

State the irreversible safeguard clearly:
> **Warning**: Local teardown is irreversible. Remote branches are evidence and will remain untouched.

Ask for explicit human teardown approval after presenting this set and warning.

### Step 5: Apply Teardown

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
  "schema_version": 2,
  "feature": "feature-cleanup",
  "branch": "feature-cleanup",
  "fingerprint": "sha256:…",
  "approvals": {
    "delivery": {
      "approved": true,
      "at": "2026-08-28T12:00:00Z"
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

- `schema_version` is `2`. Schema 1 records, which carry `approvals.documentation`, are still accepted: there `decision` is `"written"` (non-empty `paths`, null `reason`) or `"not_required"` (non-empty `reason`, empty `paths`), and every `paths` entry is hall-relative and must resolve inside `docs/memory/` or `docs/repo-relations/`. A schema 2 record with `approvals.documentation` is refused. Any other version is refused, never migrated.
- `feature` must equal the feature being cleaned; `branch` must equal that feature's recorded branch.
- `fingerprint` references the preview by hash. The preview is **not** inlined, so the record cannot drift from the value the CLI recomputes.
- `at` is an ISO-8601 UTC timestamp.
- `approvals.delivery.approved` and `approvals.teardown.approved` must both be `true`. `false` is a valid record and a refusal to apply, never a bypass.
- `outcome` is `null` until apply completes; apply writes the final per-repository and teardown results into it. A record whose `outcome` is already populated describes a finished run and is refused as an authorization for a new one.
- `outcome.descendants` lists each removed subfeature in teardown order (leaves first). It is omitted for a feature with no subfeatures, so such records are unchanged.
- The `delivery.cleaned` observation carries `docs`: the schema 1 decision, or for schema 2 `reviewed`, `skipped` or `missing` from the feature's memory review.
