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
  plans/<feature>/*.md          committed    requirements, analysis, plan
  .ivar/skills/                 committed    skills the team shares
  .ivar/setups/<repo>.sh        committed    per-repo worktree bootstrap
  .ivar/setups/<repo>.session.sh committed   per-repo session hook
  .ivar/secrets/                local        secret material you maintain by hand
  .ivar/state.json              local        hall state, health, bookkeeping
  .ivar/repos/                  local        bare clones and worktrees
  .ivar/features/               local        promotion records, execution boards
  .ivar/sessions/               local        discovery-session view dirs
  .claude/commands/ivar-*.md    local        derived workflow commands (Claude Code)
  .opencode/commands/ivar-*.md  local        derived workflow commands (OpenCode)
```

"Committed" means it belongs in your hall's git history and your teammates get it
on `git pull`. "Local" means it is gitignored, belongs to one machine, and is
reproducible — deleting it costs you a re-clone, never work.

The `ivar-*.md` workflow commands under each provider's command directory are
local derived state in the same sense: `ivar init`, `ivar provider add`, and
`ivar sync` recreate or repair them from the binary, so they are never
committed and never hand-edited. Every *other* file in those directories is
yours and is never changed.

`.ivar/secrets/` is local for a reason worth stating: the hall's `.gitignore`
excludes `.ivar/*` and negates only the committed children, so anything else
under `.ivar/` is ignored by construction. A secrets directory that depended on
someone remembering to add a line is a secrets directory that eventually leaks.
`ivar` creates the directory and never writes into it.

`ivar.json` is deliberately **not** inside `.ivar/`. It is the file a reviewer
reads in a pull request, so it stays visible at the root.

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
| `ivar.json` | 1 |

`ivar.json` starts at 1. There is no version 0 to migrate from: a file with no
`version` field is not an `ivar.json`, and is rejected rather than adopted.

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
