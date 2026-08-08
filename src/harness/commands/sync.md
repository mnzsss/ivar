---
description: Reconcile the hall's derived local state against its ivar.json.
---

Reconcile the hall's derived local state against its sources of truth: the
committed `ivar.json` and the installed `ivar` package.

## What sync does

In order:

1. The hall skeleton — `.ivar/`, `.ivar/repos/`, and the `.gitignore` lines.
2. Each repo in `ivar.json`: bare clone, default-branch worktree, setup script.
3. Each provider the hall lists: the managed block in its instruction file
   (`CLAUDE.md` / `AGENTS.md`), its MCP config, and its shipped workflow
   commands (`/ivar-*`). A provider the hall no longer lists has all three
   stripped.

The command directories (`.claude/commands/`, `.opencode/commands/`) contain
**derived state**: `ivar` owns exactly the files named `ivar-*.md`, and
reconciles them on every sync — missing or modified official commands are
restored, leftover `ivar-*` files are removed. Every other file in those
directories belongs to you and is never touched. Don't hand-author
`ivar-*.md` files.

## Steps

1. Run `ivar sync`.
2. Review the report. A missing or modified official command is restored; a
   file you wrote that does not match the `ivar-*` prefix survives untouched.

## When to run

- After editing `ivar.json` (adding or removing repos or providers).
- After updating the `ivar` package (shipped commands added, removed, or
  changed).
- When `ivar session start` warns that the config is stale.
- When `ivar doctor` reports structural degradation — sync may repair it by
  rematerialising config and repos.
