---
description: Open a feature in a multi-root VSCode workspace for cross-repo review.
---

# Review

`/ivar-review` opens a feature in a multi-root VSCode workspace for cross-repo
review.

## Usage

Open a workspace for visual review:

```bash
ivar feature review <feature>
```

`ivar feature review --help` shows the current surface.

## When to use

- You need to inspect changes across all promoted repos before delivery.
- You are the human reviewer approving or rejecting the feature.

## What happens

1. Writes `<hall>/<feature>.code-workspace` — a VSCode multi-root workspace
   with one folder per repo in the hall: a promoted repo opens on its feature
   worktree (editable), every other on its read-only default-branch worktree,
   as context.
2. You review the workspace, then decide whether the feature is ready to
   deliver.

## Requirements

- The feature must exist. That is the only precondition — there is no
  lifecycle gate, and a feature with nothing promoted still opens (every
  folder is simply read-only context).
