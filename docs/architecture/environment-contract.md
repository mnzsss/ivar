# Environment contract

Part of the [architecture overview](../../ARCHITECTURE.md).

## Environment contract

Setup scripts and anything else `ivar` spawns get these, and they are a **public
contract** — a user's committed `.ivar/setups/<repo>.sh` breaks if they move.

| variable | setup script | session hook | value |
| --- | --- | --- | --- |
| `IVAR_HALL` | ✓ | ✓ | the hall root |
| `IVAR_REPO` | ✓ | — | the repo name, as it appears in `ivar.json` |
| `IVAR_BRANCH` | ✓ | — | the branch that worktree is checked out on |
| `IVAR_WORKTREE` | ✓ | — | the absolute path of the worktree (also the cwd) |
| `IVAR_WORKTREE_KIND` | ✓ | — | `default` or `feature` |
| `IVAR_SECRETS_DIR` | ✓ | — | `.ivar/secrets/` — hand-maintained, never written by `ivar` |
| `IVAR_FEATURE` | feature worktrees only | ✓ | the feature name |
| `IVAR_SESSION_ID` | — | ✓ | the session id |
| `IVAR_SESSION_PATH` | — | ✓ | the view dir |
| `IVAR_PROVIDER` | — | ✓ | the active harness provider (`claude-code`, `opencode`, `omp`, or `t3`) |

The three worktree variables were added when slice 2 landed the setup-script
runner: a script's whole job is to bootstrap *this* repo on *this* branch, and
neither fact was derivable. `IVAR_WORKTREE` duplicates the working directory on
purpose — a script that `cd`s somewhere still needs a way back.

`IVAR_FEATURE` is set on the promote path and absent on the sync path, because
`sync` runs against the default worktree where there is no feature to name.
`IVAR_WORKTREE_KIND` is what a script branches on to know which it is in; a
script that reads `IVAR_FEATURE` unguarded should fail loudly on the default
worktree rather than quietly bootstrap the wrong thing.

`IVAR_SECRETS_DIR` points at a directory `ivar` creates and never writes into.
The tool holds no secrets — the same posture `domain::mcp` takes for MCP server
definitions. It lives under `.ivar/`, so the hall's `.gitignore` (`.ivar/*` plus
exactly two negations) covers it without a line of its own; that is the reason
for the location, not a coincidence of it.

The prefix is `IVAR_`, with **no fallback to the old `BIFROST_` names**. Same
reasoning as the manifest: this implementation is new, nobody outside the private
monorepo ever had the old prefix, and being born with compatibility debt it never
incurred is how a tool accumulates two names for everything.

### Two scripts, two lifetimes

`IVAR_SESSION_ID` is load-bearing beyond bookkeeping — it is what a hook derives
a per-session database or compose project name from, which is the only answer
`ivar` offers to shared daemon state. See `docs/reference/limitations.md`.

It reaches the **session hook**, not the setup script, and the split is forced
by the receipt. `.ivar/setups/<repo>.sh` bootstraps a *worktree*: a receipt in
the worktree's git admin directory makes it run about once, which is the only
reason `ivar sync` stays cheap enough that people keep running it. A session's
database has to come up every time a session opens, and several sessions can
share one promoted worktree — so a receipt keyed to the worktree would skip
exactly the runs that matter.

| | setup script | session hook |
| --- | --- | --- |
| file | `.ivar/setups/<repo>.sh` | `.ivar/setups/<repo>.session.sh` |
| runs on | `sync`, `promote`, `repo setup` | `session start` |
| how often | once per worktree, receipt-gated | once per session, ungated |
| failure | refuses on `sync`, warns on `promote` | warns; the session still opens |

Both are committed, both are per-repo, and both run under `bash` with the
worktree as cwd. Only the hook sees the session.
