---
description: Preview and apply delivery for a feature — push branches and create or update pull requests.
---

# Deliver

`/ivar-deliver` previews and applies delivery for a feature: it pushes branches
and creates or updates pull requests.

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

## PR metadata

Use `--name` and `--body` to set the pull request title and body.

**Global (applies to every repo):**

```bash
ivar feature deliver <feature> --preview \
  --name "feat: add session resume" \
  --body "Closes the gap in session state."
```

**Scoped (per repo):**

```bash
ivar feature deliver <feature> --preview \
  --repo api --name "feat: add user endpoint" \
  --repo web --name "feat: add user page"
```

A scoped `--name`/`--body` overrides the global value for that repo. A
scoped repo that omits `--name` or `--body` inherits the global value.

### Body values

- **Inline string:** `--body "short description"`.
- **File reference:** `--body ./notes.md` or `--body ./notes.txt` — the file
  contents are read and used as the PR body. Path is resolved relative to the
  current working directory.

### Title guidance

Use short, semantic, squash-ready titles in the pattern `<type>: <short message>` — for example `feat: add session resume` or `fix: handle empty checkout`. Avoid Linear issue identifiers in PR titles; link the issue in Linear instead.

### Land conflict

`--name` and `--body` cannot be used with `--land`. Land mode merges the
feature into the upstream branch and does not create or update pull requests,
so PR metadata is rejected.

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
- Between preview and apply, focus on the preview Repos and compare `HALL.md`'s
  relation context with the Analysis. Offer `/ivar-relations` only for concrete,
  unreflected evidence. Deferring it does not block apply and does not invalidate
  the delivery fingerprint — and this checkpoint never writes `HALL.md` directly.
