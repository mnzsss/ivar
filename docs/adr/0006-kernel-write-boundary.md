# ADR-0006 — Kernel-Enforced Write Boundary via Re-Exec Launcher

- **Status:** accepted
- **Date:** 2026-09-07

## Context

ADR-0003 established the "env by process, guard by hook" architecture. In that design,
`ivar guard` acted as a pre-tool hook to reject writes outside the session's writable
set. However, hook-based enforcement suffered from critical limitations:
1. It depended on the AI harness/provider faithfully invoking and respecting the hook.
2. It could only inspect known structured file tools (`Write`, `Edit`), leaving shell
   commands (`Bash`) and subprocesses unconstrained.
3. Hook-level heuristics required maintaining an endless matrix of harness-specific tool
   names and payload schemas.

We need a true kernel-enforced write boundary on supported operating systems without
violating Ivar's core invariant: `unsafe_code = "forbid"` (`Cargo.toml:77`).

## Decisions

### D1 — Re-exec launcher subcommand (`ivar session sandbox`)

Applying a Linux Landlock ruleset is irreversible and inherited by all future threads
and child processes. Because `ivar` must continue writing session state (`state.json`,
receipts) and driving the TUI, `ivar` itself cannot be restricted.

Because `portable-pty`'s `CommandBuilder` does not offer a `pre_exec` hook, and standard
library `pre_exec` requires `unsafe`, `ivar session start` spawns a hidden re-exec
launcher: `ivar session sandbox --session <id> -- <provider> [args...]`.
This launcher child process resolves the session's authoritative `WritableSet` from disk,
builds and applies the Landlock ruleset to itself, and then calls
`std::os::unix::process::CommandExt::exec`. Calling `exec()` is 100% safe Rust, replaces
the launcher with the provider process, and preserves the Landlock sandbox across `execve`.

### D2 — Write-only handled access rights

The Landlock ruleset declares only write-class access rights (`AccessFs::from_write`).
Read-class rights (`ReadFile`, `ReadDir`, `Execute`) are never declared as handled.
Because Landlock only restricts handled categories, all filesystem reads and binary
executions remain ambient. Promoted and unpromoted repositories alike remain readable
without explicit allowlisting.

Where supported (ABI >= 6), `Scope::Signal` and `Scope::AbstractUnixSocket` are applied
best-effort to isolate processes.

### D3 — Explicit write footprint

In addition to the session view directory, feature directory, and promoted worktrees,
the sandbox explicitly grants write rights to paths required for normal tool operation:
- The backing Git store (`.bare/`) of promoted repositories and their objects/refs.
- `/dev/null` (required for standard UNIX I/O redirection and Git operations).
- System temporary directories (`/tmp`, `$TMPDIR`).
- Provider-specific state directories (e.g. `~/.omp/`, `~/.claude/`, `~/.local/share/opencode/`).
- Shared build cache (`.ivar/cache/`) when configured.

### D4 — Advisory hook demoted to diagnostic role

`ivar guard` and harness hooks are retained on all platforms. On Linux with Landlock,
the hook fires before tool calls to provide friendly diagnostic error messages
(`writable set: ...`) rather than letting the agent encounter an uninformative raw
`EACCES`.

### D5 — Graceful degradation on non-Linux platforms

On macOS (`x86_64-apple-darwin`, `aarch64-apple-darwin`) or Linux kernels without
Landlock support (or returning `EOPNOTSUPP`), `Sandbox` detects unavailability, logs/reports
degraded status, and executes the provider without ruleset restrictions. Absence of kernel
sandboxing is never a fatal error.

### D6 — `no_new_privs` side effect

Applying Landlock without `CAP_SYS_ADMIN` automatically sets `no_new_privs` on the
sandboxed process. Descendant processes cannot acquire new privileges via setuid/setgid
binaries.

## Consequences

- The write boundary is enforced by the Linux kernel; agents cannot escape via shell
  scripts, Python one-liners, or child processes.
- Zero `unsafe` code added to the codebase.
- ADR-0003's observation that "the hook is the error message, not the barrier" becomes
  the deliberate architectural reality.
- Non-Linux platforms continue to rely on the advisory hook layer.
