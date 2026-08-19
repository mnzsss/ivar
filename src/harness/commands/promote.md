---
description: Promote a repo from read-only default-branch checkout to a writable feature-branch worktree for the current feature.
---

# Promote

`/ivar-promote` moves a repo from read-only default-branch checkout to a
writable feature-branch worktree for the current feature.

## Usage

```bash
ivar feature promote <feature> <repo>
```

`ivar feature promote --help` shows the current surface.

## When to use

- You need to modify files in a repo that is currently read-only.
- You are in a feature session and want to start editing a specific repo.

## What happens

1. **The branch.** A branch git already has is adopted and checked out as-is —
   no rebase, no reset, since those commits are someone's work. One git does
   not have is created off the feature's effective base (`--base`, then the
   feature's declared base, then the repo's `default_branch`). A declared base
   the repo does not have falls back to `default_branch` with a
   `feature.base_absent` warning.
2. **The worktree**, cut from the repo's bare clone. `ivar sync` must have
   cloned it already — promote never reaches the network.
3. **The promotion record**, written *before* the setup script runs, so a
   script that dies leaves a recorded promotion rather than a worktree nothing
   claims.
4. **The repo's Setup Script** (`.ivar/setups/<repo>.sh`), in the new worktree
   with `IVAR_WORKTREE_KIND=feature`. Failure is **non-fatal**: the repo stays
   promoted, its worktree state is recorded as `failed`, and a
   `feature.setup_script_failed` warning names the worktree that was left
   un-bootstrapped. Nothing re-runs it for you — `ivar repo setup` and
   `ivar sync` both target the default-branch worktree, not this one.
5. **Every live session of the feature is repointed**: each view dir is
   re-materialised so its symlink for this repo moves from the read-only
   default-branch worktree to the new feature worktree. A session that cannot
   be re-materialised warns (`session.not_repointed`) naming the
   `ivar session connect` that repairs it; the promotion still stands.

## Requirements

- The feature exists, the repo is declared in `ivar.json`, and the repo is not
  already promoted into that feature. Each refusal names its way out.
- `ivar sync` has cloned the repo — promotion works on the bare clone and
  never clones on its own.
- The feature's promotion set is not yet frozen by an integration receipt.
- Promotion is an **operator action** — the user or agent initiates it
  explicitly. There is no automated promotion gate.

`ivar feature promote` reads no environment: the feature and the repo are
arguments. Running it from inside a feature session is the ordinary workflow,
not a precondition of the command.
