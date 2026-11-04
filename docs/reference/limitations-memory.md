# Limitations of session capture

What session capture does not record, and what it costs. See
[Limitations and failure modes](limitations.md) for the rest of `ivar`.

Capture records four events per session into `.ivar/memory.db`: `session.started`,
`session.prompted`, `session.tool` and `session.ended`. It records an event only when
the provider's hook exposes it.

## Hook cost on every tool call

Each captured event is one `ivar memory capture` process. On Claude Code the hook
adds an entry beside the guard on every tool call. On OMP and OpenCode the plugin
spawns one process per event. The process reads at most 8 MiB of the payload, cuts
the event body to 32 KiB before redaction and 4 KiB after, and only appends a line to a file under
`.ivar/spool/memory/`; it never opens SQLite. Measured on an optimised binary,
the hook's p95 was about 15.8 ms per event. A slow disk or a debug build costs more.

## Events a provider does not expose

| Provider | Gap |
| --- | --- |
| OpenCode | No session end, so no `session.ended` is recorded. Its `tool.execute.after` reports `failed: false`, so no OpenCode `session.tool` is recorded as a failure and none gets importance 6. |
| T3 Code | `session.ended` is recorded when the thread is closed (archived) with reason `other`, not when a turn finishes, so an idle open thread has no end yet. Deleting a thread or quitting the app was not tested. T3 hosts Claude Code, so it uses the Claude Code hooks. |

The payload fixtures under `tests/cli/capture_payloads` are written by hand from each
provider's documented hook format. They were not recorded from live sessions, so a
provider that changes its payload shape is caught by `ivar doctor`
(`provider.capture_hook_missing`) only when the hook disappears, not when it changes.

## Events lost or delayed

- A session spool is capped at 8 MiB. Past the cap, events are counted in
  `<key>.dropped` and discarded. `ivar doctor` reports them as `memory.spool_dropped`
  until a drain acknowledges the count, and for 7 days after the last drop.
- Events reach `memory.db` only when a drain runs. `ivar memory status` drains on demand,
  and the `ivar graph mcp` leader drains while it runs. The leader does not drain when
  `GraphDb::open` fails, so with a broken graph database only `ivar memory status`
  drains.
- A hook that opened `<key>.jsonl` just before a drain renamed it can append to the
  renamed file after the drain read it. The window is microseconds and the event is
  lost. This is accepted.
- Sanitising redacts known secret patterns before an event reaches disk. A secret
  in a shape no pattern matches is stored as written.

## Retention

Nothing deletes observations yet. The `observations` table only grows until
the retention sweep ships; `ivar memory clean` deletes the whole database.

## Older `ivar`

A hall that sets `memory.capture` in `ivar.json` cannot be read by an `ivar` that
predates the key. See [On-disk format: versions](on-disk-format-versions.md).
