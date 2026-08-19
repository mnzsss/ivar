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

For the whole hall, run `ivar feature list` — that is also where a feature's
lifecycle state lives.

## Output

- Feature name and branch
- One line per **promoted** repo: its recorded worktree state
  (`pending` / `ready` / `failed`), whether the worktree is present on disk,
  the base the promotion was cut from, and whether that base has since
  diverged from what the feature would compute today
- With `--recursive`, the feature's whole subtree in pre-order, each entry with
  its derived integration state and blockers

What it does **not** report, and where to get it instead:

- **Read-only repos** — every repo in `ivar repo list` that is not listed here
  is read-only for this feature.
- **Lifecycle state** — `ivar feature list`.
- **Active sessions** — no command lists sessions. `ivar session connect
  --feature <feature>` connects when exactly one is live, and names every
  candidate when more than one is.
