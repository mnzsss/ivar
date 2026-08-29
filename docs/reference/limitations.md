# Limitations and failure modes

What `ivar` does not do, and what breaks when you push on it. This page exists so
you find these here rather than by hitting them.

`ivar` isolates two things: **the filesystem** and **ports**. Everything below is
something outside those two.

## Shared daemon state — the big one

A `git worktree` gives every feature its own files. It does not give them their own
Postgres.

Two features promoting the same repo get two worktrees, two dev servers, two
distinct ports — and **one shared database**. Run a migration in one and the other
one sees it. Truncate a table in one and the other one's tests fail.

`ivar` does not solve this, and the honest reason is that the only general
solution is a container per environment, which `ivar` rejects by design: the whole
mechanism is real directories on your real filesystem that your real editor and
your real `lazygit` can open.

**What `ivar` gives you instead is the hook.** Each repo can carry a **session
hook** at `.ivar/setups/<repo>.session.sh`, run on every `ivar session start`,
in that repo's worktree, with the session identifier in its environment. The
person who knows how to isolate that repo is the person who wrote it — so `ivar`
hands them the seam rather than guessing.

Note which file this is. The setup script next to it
(`.ivar/setups/<repo>.sh`) is receipt-gated and runs about once per worktree —
right for `pnpm install`, useless for a database that must come up every session.
The hook is ungated and runs every time. A hook that fails warns and the session
still opens.

Two recipes that work:

```sh
# docker compose: one project name per session, so volumes do not collide
export COMPOSE_PROJECT_NAME="${IVAR_SESSION_ID}"
docker compose up -d

# postgres: one database per session
createdb "app_${IVAR_SESSION_ID}"
export DATABASE_URL="postgres:///app_${IVAR_SESSION_ID}"
```

That is not a workaround. It is the boundary: `ivar` owns worktrees and ports;
the repo owns its own state.

## Ports are reserved, not proxied

A session gets a range of ports reserved in `state.json`, and `ivar` writes them
into the session environment. That stops another `ivar` session from being handed
the same port. It does not make a process bind correctly, and it does not proxy
traffic.

A server that ignores the assigned port can still collide with another process.
A process that dies leaves its reserved port unavailable until the session is
stopped or cleaned up. `ivar status` reports the reservation; it cannot inspect
arbitrary processes to prove the port is free.

## Writable worktrees are shared within a feature

Promoting a repo makes its feature worktree writable. Every session on that
feature points at that same worktree. This is intentional: sessions cooperate on
one branch, rather than silently diverging into per-agent branches.

It means `ivar` does not prevent two agents from editing the same file. Git
serialises individual operations; it does not resolve concurrent intent. Use
coordination, reviews, or isolated branches when you need stronger separation.

## Execution is not a sandbox or a provider controller

Ivar does not sandbox provider activity. A provider can run shell commands,
formatters, and generators against any promoted worktree available to its
session. The Run Receipt records exact baseline and final snapshot evidence for
audit, but it neither attributes individual writes nor reverts them.

The active provider, not Ivar, creates, schedules, monitors, and synthesizes
native subagents. Ivar does not launch headless provider children, parse provider
transcripts, retain conversation or native-subagent identifiers, or promise that
a logical resume restores provider context. Resuming a receipt with another
provider attaches the current Feature Session to the same local audit record.

Use repository permissions, isolated environments, or a platform sandbox when
you need stronger write isolation.

## Local state is disposable

Folders like `skills/`, `skills-local/`, and `setups/` are managed or personal;
everything else under `.ivar/` is local derived state.
Removing it costs a re-clone and session recreation, not committed work. Do not
put source files, plans, secrets you need to share, or irreplaceable artifacts
there.

## A feature can span only registered repos

`ivar` cannot promote a repository it does not know about. Register it first with
`ivar repo add`; a path that merely happens to contain a Git checkout is not part
of the hall.

## Session write guard is best-effort

The session write guard (`ivar guard`) blocks structured writes (Write, Edit) that
target paths outside the session's view dir or promoted worktrees. It is a hook —
the provider invokes it before each tool call — and its effectiveness depends on
the provider respecting the hook protocol:

- **Claude Code:** The guard always exits 0; the deny decision travels in the JSON
  body. If the provider ignores the `permissionDecision: deny` response, the write
  proceeds.
- **OpenCode:** The guard exits non-zero for deny. A non-zero exit should abort the
  tool call, but this is provider behaviour, not ivar's guarantee.

**What this means in practice:** The guard is an error message, not a firewall. A
provider that does not call the hook, or that ignores a deny decision, will allow
the write. The filesystem-level guarantee — non-promoted worktrees held read-only
by the kernel — remains the hard boundary. The guard catches the common case
(Write/Edit through the provider's tool interface) but cannot prevent a shell
command from writing to arbitrary paths.
