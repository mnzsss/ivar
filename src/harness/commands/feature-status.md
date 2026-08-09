---
description: Check which repos are promoted and which are read-only for a feature.
---

# Feature Status

`/ivar-feature-status` checks which repos are promoted and which are read-only
for a feature.

## Usage

```bash
ivar feature status <feature>
```

Or, when `IVAR_FEATURE` is set:

```bash
ivar feature status "$IVAR_FEATURE"
```

For the whole hall, run `ivar feature list`.

## Output

- Feature name and branch
- Lifecycle state
- List of promoted (read-write) repos
- List of read-only repos
- Active sessions for this feature
