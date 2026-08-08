---
description: Open a feature in a multi-root VSCode workspace for cross-repo review.
---

# Review

Open a feature in a multi-root VSCode workspace for cross-repo review.

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

1. Generates a VSCode multi-root workspace file with promoted repos as
   editable worktrees and non-promoted repos as read-only context.
2. You review the workspace, then decide whether the feature is ready to
   deliver.

## Requirements

- The feature must exist and have at least one promoted repo.
- The feature must be in the appropriate lifecycle state for the operation.
