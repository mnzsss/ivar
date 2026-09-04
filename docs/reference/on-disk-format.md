# On-disk format

`ivar` keeps everything it knows in files, on purpose. This page is the contract
for those files: what is committed, what is local, and what happens when the
format changes.

## The promise

> **There will never be a hall you cannot open.**

That is one sentence and it is narrower than it looks. It does **not** say the
format will not change — at `0.x` it will. It says two things:

- Every version that changes the format ships the migration for it.
- **The migration chain is never pruned.** It always starts at the beginning, so a
  hall written by any past version still opens with any later one.

A version *newer* than your binary is the one case `ivar` refuses outright. It
names both versions, tells you to upgrade, and **does not touch the file**. A
half-understood state file is worse than no state file.

## What lives where

```
<hall>/
  ivar.json                     committed    the hall's identity and its repos
  HALL.md                       committed    the canonical standing instructions
  CLAUDE.md AGENTS.md           committed    provider root aliases — relative
                                             symlinks to HALL.md (CLAUDE.md for Claude
                                             Code; AGENTS.md for OpenCode and OMP)
  plans/<feature>/*.md          committed    requirements, analysis, plan
  .ivar/skills/                 committed    skills the team shares
  .ivar/setups/<repo>.sh        committed    per-repo worktree bootstrap
  .ivar/setups/<repo>.session.sh committed   per-repo session hook
  .ivar/secrets/                local        secret material (hand-maintained files, plus mcp.env)
  .ivar/state.json              local        hall state, health, bookkeeping
  .ivar/repos/                  local        bare clones and worktrees
  .ivar/features/               local        promotion records, Run Receipts
  .ivar/sessions/               local        discovery-session view dirs
  .claude/commands/ivar-*.md    local        derived workflow commands (Claude Code)
  .opencode/commands/ivar-*.md  local        derived workflow commands (OpenCode)
  .omp/commands/ivar-*.md       local        derived workflow commands (OMP)
  .claude/skills/               local        derived materialised skills (Claude Code)
  .opencode/skills/             local        derived materialised skills (OpenCode)
  .omp/skills/                  local        derived materialised skills (OMP)
  mcp.json                      local        derived MCP configuration (OMP root)
```

"Committed" means it belongs in your hall's git history and your teammates get it
on `git pull`. "Local" means it is gitignored, belongs to one machine, and is
reproducible — deleting it costs you a re-clone, never work.

### `HALL.md` and the provider aliases

`HALL.md` is the **only editable, committed source** of the hall's standing
instructions. It belongs to the user; `ivar` owns exactly the bytes between its
`<!-- ivar:managed:start -->` and `<!-- ivar:managed:end -->` markers, plus the
`/ivar-relations` region's own markers (managed by that workflow, never by
Each enabled provider's root alias — `CLAUDE.md` for Claude Code,
`AGENTS.md` for OpenCode and OMP — is a committed **relative symlink to `HALL.md`**.
OMP shares `AGENTS.md` with OpenCode; if both are configured, the single
physical alias is jointly owned and remains as long as either provider is active.
Aliases are never sources and never workflow edit targets.

- `ivar init` creates `HALL.md` and the first provider's alias; `ivar provider
  add` creates the new provider's alias; `ivar sync` repairs the topology.
- An enabled provider whose alias is a **regular file** is never overwritten —
  `ivar sync` reports an adoption warning and preserves it byte for byte until
  a human consolidates its instructions into `HALL.md` and removes it.
- A provider **absent** from `providers.available` has its alias path entirely
  managed by `ivar`: the next `sync` removes whatever is there, including a
  regular file. This is deliberately destructive — never remove `HALL.md`.

Session view dirs also carry a provider-native instruction file, but that file
is **derived** from `HALL.md` (canonical content, plus the session bootstrap
for feature sessions), lives in the ephemeral view dir, and is never
committed.

The `ivar-*.md` workflow commands under each provider's command directory
(`.claude/commands/`, `.opencode/commands/`, `.omp/commands/`), MCP documents
(`.mcp.json`, `opencode.json`, root `mcp.json`), and guard artifacts
(`.claude/settings.json`, `.opencode/plugins/ivar.js`, `.omp/hooks/pre/ivar.js`) are
local derived state: `ivar init`, `ivar provider add`, and
`ivar sync` recreate or repair them from the binary, so they are never
committed and never hand-edited. Every *other* file in those directories is
yours and is never changed.

`.ivar/secrets/` is local for a reason worth stating: the hall's `.gitignore`
excludes `.ivar/*` and negates only the committed children, so anything else
under `.ivar/` is ignored by construction. A secrets directory that depended on
someone remembering to add a line is a secrets directory that eventually leaks.
General files under `.ivar/secrets/` are maintained by hand; Ivar manages
durable local MCP OAuth client secrets directly in `.ivar/secrets/mcp.env` with
owner-only permissions (`0600` on Unix).

`ivar.json` is deliberately **not** inside `.ivar/`. It is the file a reviewer
reads in a pull request, so it stays visible at the root.

## Run Receipt history

Execution state is local under `.ivar/features/<feature>/execution/`:

```
execution/
  run.json                         current Run Receipt, when one exists
  archive/runs/<run-id>.json       immutable terminal Run Receipts
  archive/boards/<hash>.json       immutable legacy execution evidence
```

`run.json` is the feature's single-Run lock only while its status is `active`,
`blocked`, or `diverged`. A terminal receipt is moved whole to `archive/runs/`
and `run.json` is removed, making room for the next Run. `ivar feature close`
preserves this directory and refuses while a non-terminal receipt holds the
lock.

A receipt contains the approved plan fingerprint, an immutable baseline
snapshot, coordinator session/provider lineage, checkpoints, a structured report
when supplied, and exact final snapshot evidence. It is an audit record, not a
provider transcript, subagent registry, or scheduler state.

Older local execution records are migrated on read. Ivar archives the original
legacy record under `archive/boards/` and creates a provider-neutral receipt. A
legacy completed record retains its known outcome; a legacy non-terminal record
becomes `interrupted`, because its prior coordination state cannot be resumed.
The migration is local, restartable, and never changes a repository or remote.

## Two migration policies

The split matters, and it is not a detail.

| | local state | `ivar.json` |
| --- | --- | --- |
| when read at an older version | migrates, and saves the migrated form | reads fine, saves nothing |
| when written | always current | refuses if the file on disk is older |
| how it moves forward | by itself, silently | `ivar migrate`, run by you |

### Why `ivar.json` never migrates itself

It is committed and shared. If upgrading `ivar` quietly rewrote it, that rewrite
becomes a commit — and a teammate still on the older binary would then refuse your
commit as "a version I do not understand".

One person's upgrade would break someone else's checkout. So migrating a shared
file is a **team event**, and a human decides when it happens:

```sh
ivar migrate          # shows what would change, then asks
```

Local state has no such problem. Nobody reviews it, nobody shares it, and it
migrates without telling you because there is nothing useful to say.

## Current versions

| file | version |
| --- | --- |
| `ivar.json` | 3 |
| `.ivar/features/<feature>/feature.json` | 3 |

`ivar.json` starts at 1. There is no version 0 to migrate from: a file with no
`version` field is not an `ivar.json`, and is rejected rather than adopted. A
migration chain may begin at its format's earliest supported version — `ivar.json`'s
chain is `[1→2, 2→3]` — and v0 stays unreachable.

### `ivar.json` v2

v2 adds two things, both with embedded defaults so v1 files migrate cleanly:

```json
{
  "version": 2,
  "integration": { "via": "local", "strategy": "squash" },
  "repos": [
    { "name": "api", "url": "git@github.com:acme/api.git",
      "default_branch": "main", "checks": ["cargo fmt --check", "cargo test"] }
  ]
}
```

`integration` is the hall's per-field default (CLI > feature > hall > embedded)
and `checks` are the repo's ordered verification commands. Migrating `ivar.json`
from v1 is explicit — `ivar migrate` — because the file is committed.

### `ivar.json` v3

v3 adds one optional field to an `mcp` entry: `oauth`, a pre-provisioned OAuth
client registration for a server whose host rejects a harness's own dynamic
client registration (`ivar mcp auth`'s pre-registration step; `mcp.figma.com`
today).

```json
{
  "version": 3,
  "mcp": [
    {
      "name": "figma",
      "type": "sse",
      "url": "https://mcp.figma.com/mcp",
      "oauth": { "client_id": "…", "client_secret_env": "IVAR_MCP_ACME_FIGMA_SECRET" }
    }
  ]
}
```

`client_id` is not a secret. `client_secret_env` is the *name* of an
environment variable — never a value — the same reference convention `env`
already follows elsewhere in an `mcp` entry. The field is absent by default,
so v2 → v3 adds nothing to a hall that never used it; migrating is still
explicit (`ivar migrate`) because the file is committed.

The `name` stored here (`figma`, above) is canonical and unqualified. Providers
never see it directly: every provider boundary — the key in `.mcp.json` and
`opencode.json`, the argument to `claude mcp login` / `opencode mcp auth`, the
key OpenCode writes into `mcp-auth.json`, and the OAuth secret variable name —
receives `<hall>-<server>` instead, derived from the hall this manifest belongs
to (`acme-figma`, in a hall named `acme`). This is a breaking change from
previous behaviour, where the stored name was used verbatim at every provider
boundary. A hall whose entry is already hall-qualified (`figma-acme`, say)
must rename it to the canonical form by hand and re-run `ivar sync`, and
re-export any `client_secret_env` secret under the new variable name — `ivar`
will not rewrite a committed, hand-edited file by guessing at intent.

### `feature.json` v3

v3 adds the nested-subfeature fields. Only the child side of the lineage is
stored — `parent` — and children are derived by scanning:

```json
{
  "version": 3,
  "name": "checkout-tax",
  "branch": "checkout-tax",
  "parent": "checkout-v2",
  "base": "checkout-v2",
  "integration": { "via": "pr" },
  "promotions": {
    "api": {
      "worktree": "ready",
      "base": "checkout-v2",
      "integration_receipt": {
        "source_sha": "…", "target_branch": "checkout-v2", "result_sha": "…",
        "via": "pr", "strategy": "squash", "pr_url": "https://github.com/…/pull/7",
        "verification": {
          "command_fingerprint": "…",
          "child": [ { "command": "cargo test", "success": true, "exit_code": 0, "diagnostic": "" } ],
          "parent": [],
          "pr_checks": [ { "name": "ci", "bucket": "pass" } ],
          "verified_at": "2026-08-14T12:00:00Z"
        }
      }
    }
  }
}
```

`parent` and `integration` default to absent, and `integration_receipt` to null,
so v2 files migrate in place — local state migrates silently on read. No feature
stores a child list, and no lifecycle field is persisted: the integration state
is derived from the close record plus receipt freshness. Child branches and
worktrees are retained after integration so receipt validation stays exact.

## Strictness

Config parsing is strict. An unknown key in `ivar.json` is a **hard error naming
the key**, not a warning and not silence.

This is on purpose, and it is the one place `ivar` is deliberately less forgiving
than it could be. `ivar.json` is hand-edited by people. Silently ignoring a typo
is how a team ends up with a setting that has never done anything, and nobody
finds out for months.

## Reading it yourself

Everything here is JSON or Markdown, written with sorted keys, two-space indent
and Unix line endings — so `jq` works, `git diff` is readable, and a diff only
appears when something actually changed.

There is no database, no cache you need to know about, and no format that needs
`ivar` to read it. That is the point: if this tool went away tomorrow, your work
is still on disk and still in git.
