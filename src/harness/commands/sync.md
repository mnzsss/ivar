---
description: Reconcile the hall's derived local state against its ivar.json.
---

# Sync

`/ivar-sync` reconciles the hall's derived local state against its sources of
truth: the committed `ivar.json` and the installed `ivar` package.

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
- When `ivar doctor` or `ivar status` reports the hall **degraded** — a repo
  never cloned, a default worktree gone — since sync is what rematerialises
  config and repos.

A hall reported **stale** is a different problem: it means a repo is behind its
remote, and `ivar repo pull` is what catches it up. Sync does not fetch.
