# ADR-0008 — Capture hook separate from the guard

- **Status:** accepted
- **Date:** 2026-11-04

## Context

`ivar guard` runs before a tool call and decides: it can deny the call. Session
capture runs after the fact and only observes. It needs events the guard never sees
(session start, prompt, session end) and the result of the tool call.

Capture also runs on every tool call, so it must not slow the agent down or fail
a session. SQLite writes take `memory.lock`, can block behind a drain or a graph
indexer, and can fail on a corrupt file.

## Decisions

### D1 — A separate `ivar memory capture` hook

Capture is its own hook entry per provider, not an extra branch of the guard. The
guard keeps one job, its output protocol stays untouched, and turning capture off
removes only the capture entries. `ivar memory capture --provider <name>` reads
the provider payload on stdin, always exits 0 and prints nothing.

### D2 — Spool first, drain later

The hook appends one JSON line to `.ivar/spool/memory/<key>.jsonl`. A drain
renames each file to `<key>.<nanos>.draining`, inserts its rows into `observations`
and removes the file only after the rows commit. A crash replays the file, and the
unique `event_id` absorbs the repeat. A file past 8 MiB drops the new line and counts
it in `<key>.dropped`. `_drain.json` keeps the last drain time and error.

Writing straight to SQLite from the hook was rejected: the hook would wait on
`memory.lock`, and a locked or corrupt database would cost the session its events.

### D3 — The hook never opens SQLite

Only a drain opens `memory.db`. Drains run on `ivar memory status` and in the
`ivar graph mcp` leader, so the per-tool-call path stays a file append.

### D4 — Sanitise before disk

The hook redacts secret patterns before the line is written, so a secret never rests
in the spool. The patterns are ported from ai-memory v3.3.3 (MIT), credited in `NOTICE`.
The drain sanitises again, which covers lines written by an older hook.

A drain renames each `.jsonl` to `.draining` before it reads it. A hook that opened
the `.jsonl` just before the rename can still append to the already-read file, and
that event is lost. The window is microseconds and accepted.

The installed Claude Code command is `ivar memory capture --provider claude-code`
followed by the shell fallback `|| true`.
Claude Code treats exit 2 from a `UserPromptSubmit` hook as a block that erases the
prompt, and an `ivar` older than the synced settings exits 2 on the unknown subcommand.

### D5 — On by default, one switch

`memory.capture` in `ivar.json` is on unless set to `false`. The key adds no
manifest version. `ivar sync` installs or removes the hooks to match it.

## Consequences

- Claude Code gets more hook entries per tool call. OMP and OpenCode spawn one process
  per event.
- Events wait in the spool until a drain; `ivar doctor` reports orphaned files,
  dropped events and failed drains.
- An `ivar` older than the `memory` key rejects `ivar.json`.
- A provider that does not expose an event records none for it; see
  [limitations of session capture](../reference/limitations-memory.md).
- Observations are never deleted until the retention sweep ships.
