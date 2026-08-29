---
description: Document a delivered feature, capture human approvals, and clean up local feature resources safely.
argument-hint: <feature-name>
---

# Feature Cleanup

`/ivar-feature-cleanup` orchestrates the human-gated end-of-feature cleanup process. It inspects a delivered feature, presents delivery evidence, proposes and updates durable product and update documentation, collects human approvals across three gates, writes a durable JSON cleanup record, and performs safe, deterministic local teardown using `ivar feature cleanup`.

## Usage

Preview the cleanup state without side effects:

```bash
ivar feature cleanup <feature> --preview
```

Apply an approved cleanup record:

```bash
ivar feature cleanup <feature> --record docs/updates/<NNN>-<feature>.cleanup.json
```

## Prerequisites

- You must be inside a **hall** (`ivar status` succeeds).
- The target feature must exist in `ivar feature list`.
- Feature sessions (`$IVAR_SESSION_TYPE == "feature"`) are supported, but **discovery sessions** (`$IVAR_SESSION_TYPE == "discovery"`) are rejected.

## Feature Resolution

1. Resolve the feature name from `$ARGUMENTS`.
2. If `$ARGUMENTS` is empty, fall back to `$IVAR_FEATURE`.
3. If neither is set, ask which feature to clean up.
4. Refuse discovery sessions and unknown features conversationally before gathering evidence.

## Process Overview & Approval Gates

The cleanup workflow owns three explicit human decisions, recorded independently. Present **one question per turn** with no bundled consent:

1. **Delivery Gate** — Approve evidence that every promoted repository is delivered.
2. **Documentation Gate** — Approve the exact `HALL.md` and `docs/*` diff, or explicitly approve `not_required` with a human reason.
3. **Teardown Gate** — Approve the exact local sessions, worktrees, metadata, plans, and local branches to be removed.

## Workflow Steps

### Step 1: Feature Resolution & Preview Evidence

Run `ivar feature cleanup <feature> --preview` to gather local state and evidence without side effects:
- Check reachability of feature HEAD from effective base in each promoted repo.
- Collect local branch presence, worktree status, active sessions, descendants, dirty worktrees, missing clones, and paths to be removed.
- Recompute the preview SHA-256 fingerprint over canonical JSON.

### Step 2: Delivery Gate

Present delivery evidence for every promoted repository in manifest order:
- Show commit reachability from effective base and PR status (if available).
- Highlight empty features explicitly.
- Ask the human for explicit delivery approval:
  "Do you approve that feature '<feature>' is delivered across all promoted repositories?"
- If unmerged commits or missing repos exist as delivery blockers, explain them clearly and stop.

### Step 3: Documentation Gate

Read the feature plan (`plans/<feature>/plan.md`), per-repo commits/diffs for promoted repos, `HALL.md`, and existing topics in `docs/product/`, `docs/updates/`, and `docs/repo-relations/`.

Propose the minimum durable documentation:
- **Product documentation** (`docs/product/NNN-<slug>.md`): Update an existing product topic before creating a duplicate. Describe current durable product behavior.
- **Update documentation** (`docs/updates/NNN-<slug>.md`): Add an update topic only when the delivery matters to future users or operators.
- **Repository relations**: Delegate changed repository relations to the established `/ivar-relations` contract. Never rewrite relation prose yourself.
- **Not required**: Allow an explicit `not_required` decision with a human reason (e.g. internal refactor with no user-visible change).

Rules for documentation writes:
- Show the exact documentation diff to the human.
- Write **nothing** until the documentation gate is explicitly approved.
- Re-read every target file immediately before writing and stop on concurrent change.
- Numbers are monotonic within each directory (`max + 1`, three digits, e.g. `001`, `002`). Never reuse or renumber identifiers.
- Touch only bytes inside the workflow-owned HALL index markers (`<!-- ivar:product-docs:start -->` and `<!-- ivar:updates:start -->`). Never touch `ivar-managed` or `relations` markers.
- Edit `HALL.md` directly. `CLAUDE.md` and `AGENTS.md` are aliases to `HALL.md`, never independent edit targets.

### Step 4: Record Creation & Re-preview

Write the durable cleanup record to `docs/updates/<NNN>-<feature>.cleanup.json`.

After documentation writes, re-run `ivar feature cleanup <feature> --preview` because hall state or Git refs may have changed, and bind the final record to that new fingerprint.

### Step 5: Teardown Gate

Present the exact teardown set:
- Local sessions to be stopped/removed.
- Worktrees, metadata, and plan paths to be removed.
- Local feature branches to be deleted per promoted repo.

State the irreversible safeguard clearly:
> **Warning**: Local teardown is irreversible. Remote branches are evidence and will remain untouched.

Ask for explicit human teardown approval after presenting this set and warning.

### Step 6: Apply Teardown

Update `approvals.teardown.approved` to `true` in the cleanup record and execute:

```bash
ivar feature cleanup <feature> --record docs/updates/<NNN>-<feature>.cleanup.json
```

On partial failure:
- Preserve the cleanup record with its partial outcome log.
- Explain the failure and instructions to retry.
- Never turn a failed apply into an assertion of success.

## Product Documentation & Index Markers

`HALL.md` maintains two workflow-owned index regions outside `<!-- ivar:managed:* -->` and `<!-- ivar:relations:* -->`:

```markdown
<!-- ivar:product-docs:start -->
## Product documentation

- [Feature Cleanup](docs/product/003-feature-cleanup.md): Human-gated cleanup and documentation workflow.
<!-- ivar:product-docs:end -->

<!-- ivar:updates:start -->
## Updates

- [Feature Cleanup](docs/updates/007-feature-cleanup.md): Delivered the feature-cleanup command and enforcement CLI.
<!-- ivar:updates:end -->
```

Files live under `docs/`:
- `docs/product/NNN-<slug>.md` — current durable product behavior.
- `docs/updates/NNN-<slug>.md` — delivery-relevant change log.
- `docs/repo-relations/NNN-<slug>.md` — owned exclusively by `/ivar-relations`.

## Cleanup Record Schema

The durable cleanup record is stored at `docs/updates/<NNN>-<feature>.cleanup.json`.

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
        "docs/product/003-feature-cleanup.md",
        "docs/updates/007-feature-cleanup.md"
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
