# Build order and non-goals

Part of the [architecture overview](../../ARCHITECTURE.md).

## Build order

Vertical slices — one full journey at a time, end to end — not layer by layer.
This is what the spike did, and it is why 51 of 52 differential cases passed on
first assembly instead of surfacing at integration.

```
1  ivar init                 store, manifest, layout, canonical json, error envelope
2  ivar sync                 harness config materialisation, setup scripts, git clone
3  ivar repo …               the Git trait both ways, worktrees, ro_guard
4  ivar feature …            promotion, branch resolution, typestate worktree creation
5  ivar session start        view dir, harness spawn, TUI, PTY, emulator
6  ivar plan …               SPDD artifacts, approval gates
7  ivar skill …              mostly ported already by the spike
8  status · doctor · cleanup · migrate   reconciliation and hygiene, informed
                             by everything above
```

Slices 1–2 are where the foundations get set, so they are the expensive ones:
after those, the error envelope, the warning discipline, the canonical writer and
the versioned store all exist and every later slice is cheaper than the last.

Three items are launch gates rather than polish, and they attach to specific
slices: port attribution (slice 5 — two repos and two dev servers collide on the
*first* session otherwise), the per-repo setup script (slice 2 — a git worktree
shares history but not untracked files, so a fresh worktree has no `.env` and no
`node_modules`), and testing the view dir against a third-party git TUI (slice 3 —
the closest prior art broke `lazygit` outright with output that was perfectly
legal).

## What is deliberately not here

- **No async runtime, no daemon, no server, no socket, no telemetry.** The
  local-only claim is verifiable by `rg` over this repo, and it should stay that
  way. Every network call is one you asked for (`deliver`, `mcp auth`, a
  tarball fetch) — with **one named exception**: the update check
  (`src/infra/release.rs`). At most once per 20 hours, and only on an
  interactive release build, `ivar` asks GitHub where
  `github.com/mnzsss/ivar/releases/latest` redirects, caches the answer in
  `$XDG_CACHE_HOME/ivar/update-check.json`, and prints one stderr line when a
  newer release exists. It sends nothing but a `User-Agent`, never blocks a
  command for more than 800 ms, and `IVAR_NO_UPDATE_CHECK=1` turns it off.
- **No Ivar-owned provider scheduler.** Provider-native subagents, dependency
  coordination, transcripts, event folding, and per-subagent write contracts
  belong to the active provider. Ivar keeps only the local Run Receipt boundary.
- **No MCP surface yet.** Deferred with the cost named: the closest prior art
  shipped 67 MCP tools and had to cut to 15 after finding the agent spent its
  context orchestrating instead of reading code. Marking a verb costs an
  annotation; unmarking 67 tools costs a month.
- **No container per environment.** `ivar` isolates the filesystem and ports. It
  does **not** isolate a daemon with shared state — a shared Postgres stays
  shared. The mechanism offered instead is the setup script hook plus a documented
  recipe, and the limitation is named on a page users find before they hit it.
- **No Windows.** The view dir is built entirely from symlinks, which need
  Developer Mode or admin rights. WSL is untested.
