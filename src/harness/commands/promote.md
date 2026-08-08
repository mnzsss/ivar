---
description: Promote a repo from read-only default-branch checkout to a writable feature-branch worktree for the current feature.
---

# Promote

Promote a repo from read-only default-branch checkout to a writable
feature-branch worktree for the current feature.

## Usage

```bash
ivar feature promote <feature> <repo>
```

`ivar feature promote --help` shows the current surface.

## When to use

- You need to modify files in a repo that is currently read-only.
- You are in a feature session and want to start editing a specific repo.

## What happens

1. Creates a feature-branch worktree for the repo (if it does not exist).
2. Runs the repo's Setup Script (`.ivar/setups/<repo>.sh`) the first time a
   fresh worktree is materialised. Setup failure is **non-fatal** — the repo
   stays promoted but a warning is printed.
3. Repoints every live session's symlink for this repo from the default branch
   to the feature branch.
4. Updates the feature state to track the promotion.

## Requirements

- Must be in a feature session (not a discovery session).
- `IVAR_SESSION_ID` and `IVAR_FEATURE` env vars must be set.
- Promotion is an **operator action** — the user or agent initiates it
  explicitly. There is no automated promotion gate.
