---
description: Preview and apply delivery for a feature — push branches and create or update pull requests.
---

# Deliver

Preview and apply delivery for a feature. Pushes branches and creates or updates
pull requests.

## Usage

Generate a side-effect-free delivery preview:

```bash
ivar feature deliver <feature> --preview
```

Apply an approved delivery, pinned to the fingerprint the human reviewed:

```bash
ivar feature deliver <feature> --fingerprint <fp>
```

Run `ivar feature deliver <feature> --help` for the full flag surface.

## When to use

- The feature has been reviewed and approved.
- You want to preview what branches would be pushed and which PRs created.
- You are ready to push code and create or update PRs.

## What happens

1. **Preview** (`--preview`, side-effect-free): reads local state for each
   promoted repo — branch name, remote, base branch, existing PR status, HEAD
   and base SHAs — and computes a content-based fingerprint to detect state
   drift. Nothing is pushed.
2. **Apply** (`--fingerprint <fp>` required): validates the supplied
   fingerprint from the reviewed preview against current state — rejects if
   state changed. Never replaces the reviewed fingerprint with a freshly
   generated one. Pushes each accepted repo's branch and creates or updates
   its PR.

## Important

- **`--fingerprint` is required to apply.** Pass the fingerprint printed by the
  reviewed `--preview` output. Apply never generates a new fingerprint — it
  must match the human-reviewed preview.
- Never auto-merges — `ivar` only observes merged state.
- A stale preview fingerprint (state changed since generation) is rejected —
  generate a fresh preview.
