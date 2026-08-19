---
description: List the repositories registered in the hall manifest and their local state.
---

# Repo List

`/ivar-repo-list` lists the repositories registered in the hall manifest and
their local state.

## Usage

```bash
ivar repo list
```

## Output

`ivar repo list` shows every repo in `ivar.json` with its name, default branch,
and URL.

Neighbouring views, each of which answers a different question:

| Question | Command |
|---|---|
| Is the hall's local state healthy — is each repo cloned and materialised? | `ivar status` |
| What features exist, on which branch, how many repos promoted, what lifecycle state? | `ivar feature list` |
| Which repos are promoted into one feature, and what state is each worktree in? | `ivar feature status <feature>` |

`ivar status` reports hall health and, per repo, whether its bare clone and
default worktree exist. It does **not** list features, sessions, or promotions
— use the commands above for those.
