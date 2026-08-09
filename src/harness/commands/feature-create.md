---
description: Create a new persistent feature for cross-repo work.
---

# Feature Create

`/ivar-feature-create` creates a new persistent feature for cross-repo work.

## Usage

```bash
ivar feature create <name>
```

## What happens

1. Creates `.ivar/features/<name>/` with the feature's state.
2. Sets the branch name (defaults to `feat/<name>`).
3. No repos are promoted yet — use `ivar feature promote <feature> <repo>` to
   enable writes.

## When to use

- Transitioning from a discovery session to implementation.
- Starting a new cross-repo feature from inside an active session.

## Critical

**Never create `.ivar/features/` directories or state files manually.** Always
use the CLI command. The schema is validated at runtime — hand-crafted files
with wrong fields will be silently ignored.
