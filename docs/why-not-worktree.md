# Why not just `git worktree` and a script?

This is the right question, and the honest answer starts by conceding most of it:
**you can build the happy path yourself, in an afternoon.** `git worktree add`
per repo, a loop over a list of URLs, a `cd`. If that is all you need, it is all
you need, and you should not adopt a tool for it.

What follows is what the afternoon version does not give you. Each item is
something that goes wrong in practice, not a feature list.

## 1. Nothing stops the agent writing to the wrong repo

This is the big one, and it is why the tool exists.

Your feature touches `api` and `web`. `docs`, `infra` and four other repos are
sitting right there in the same tree. An agent asked to "update the client" has
no way to know that `infra` is off-limits, and a sufficiently confident one will
edit it, commit it, and tell you it is done.

`ivar` clears the write bits on every worktree the feature has not promoted. Not
a config setting, not a hook, not a prompt asking the model to behave —
`chmod`, enforced by the kernel. The agent gets `EACCES` and an error message
naming the way out (`ivar feature promote docs`), and the decision to widen the
blast radius stays yours.

A hand-rolled script can do this too. It is just that nobody's does, and the
failure is silent until it isn't.

## 2. A worktree is not a working directory

`git worktree add` gives you the tracked files. It does not give you `.env`,
`node_modules`, `target`, a built `dist/`, or a seeded database — everything
untracked, which is everything that makes the checkout runnable.

So the afternoon version produces a directory that looks right and does not
build. Then it produces it again, per repo, per feature.

`ivar` makes that a **setup script per repo**, committed to the hall, run when a
worktree is first materialised — during `sync` and on first `promote`. It is
gated by a receipt owned by that physical worktree, so it runs once and not on
every command, and re-runs when the script itself changes.

The script is also the answer to the thing `ivar` genuinely cannot isolate: a
shared database. It is the hook where you derive a per-session schema from
`IVAR_SESSION_ID`. See [Limitations](reference/limitations.md).

## 3. Two dev servers, one port

Two features open at once, two `web` worktrees, both `npm run dev`, both port
3000. The second fails, or worse, the first silently serves the second's code.

`ivar` attributes listening ports to the process that opened them, so a session
can tell you which port belongs to which repo instead of leaving you to guess.

## 4. The read-only trick breaks other tools if you do it naively

The closest prior art to this idea — a Rust multi-repo worktree manager — broke
`lazygit` outright, with output that was perfectly legal git. Once you start
assembling directories out of symlinks and read-only trees, the tools your team
already runs inside them become the test surface.

`ivar` has a test that opens `lazygit` against a real view dir, because finding
this out from a bug report is expensive and finding it out from CI is not.

## 5. Onboarding is the actual product

The afternoon script lives on the machine of the person who wrote it. The hall is
a git repo: `git clone && ivar sync` gives a new teammate every repo, on the
right branches, with each one's setup script run.

That is the difference between a script and a shared definition, and it is most
of the value if your team is larger than one.

## When you genuinely should not use this

- **One repo.** Use `git worktree` directly. This tool's entire subject is the
  space *between* repos.
- **You need Windows.** The view dir is built from symlinks. Use WSL, or don't.
- **You want isolation of running services, not files.** A container per
  environment is a different tool and a stronger promise. `ivar` isolates the
  filesystem and ports, and says so.

## The one-line version

`git worktree` gives you many working copies of one repo. `ivar` gives you one
working directory across many repos, with the ones you did not ask for held
read-only — and gives the whole arrangement to your team as a committed file.
