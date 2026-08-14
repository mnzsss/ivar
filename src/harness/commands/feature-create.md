---
description: Create a new persistent feature for cross-repo work.
---

# Feature Create

`/ivar-feature-create` creates a new persistent feature for cross-repo work.

## Usage

```bash
ivar feature create <name>
```

A **subfeature** is created with a parent, which derives its base from the
parent's branch:

```bash
ivar feature create <child> --parent <current>
```

## What happens

1. Creates `.ivar/features/<name>/` with the feature's state.
2. Sets the branch name (defaults to `feat/<name>`).
3. No repos are promoted yet — use `ivar feature promote <feature> <repo>` to
   enable writes.
4. With `--parent <feature>`, the child's `base` is the parent's branch, and
   only the child-side `parent` fact is stored — children are derived by
   scanning, never listed on the parent.

## When to use

- Transitioning from a discovery session to implementation.
- Starting a new cross-repo feature from inside an active session.
- **As the coordinator**: automatically create a child for an isolatable
  request that falls **outside the approved plan**. Run
  `ivar feature create <child> --parent <current>` yourself, then **announce**
  the new child — do not ask permission. The executor never creates features;
  it stops and reports, and you create.

## Critical

**Never create `.ivar/features/` directories or state files manually.** Always
use the CLI command. The schema is validated at runtime — hand-crafted files
with wrong fields will be silently ignored.
