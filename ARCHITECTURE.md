# Architecture

How `ivar` is put together, and the rules that keep it that way. For *why* each
dependency was chosen, see [ADR-0001](docs/adr/0001-stack-and-tooling.md).

## The one-sentence model

`ivar` assembles a directory. Everything else serves that: a **Hall** owns N
**Repos** as bare clones; a **Feature** is one branch across the repos it has
**Promoted**; a **Session** materialises a **View Dir** of symlinks into exactly
those worktrees and opens a harness in it. The repos a feature has not promoted
are guarded read-only, kernel-enforced on Linux.

Two properties fall out of that and constrain every module below.

**The work does not live inside a vendor.** Hall, feature, branch, promoted repos,
view dir and plan are files on disk and commits in git. A session dying loses the
conversation and nothing else. So: no state may exist only in a running process,
and no verb may require a live session to be useful.

**Read-only is enforced below the harness, not by it.** Non-promoted worktrees
have their write bits cleared (`mode & ~0o222`) on the worktree root — one path,
never the tree below it, for a reason `docs/reference/limitations-delivery.md` spells out.
On Linux with Landlock, an agent launched by `ivar session start` with a TTY runs
in a sandbox where the kernel refuses writes outside the promoted repos; on macOS,
on kernels without Landlock, under `ivar session connect` and in a `t3` session
(its agent is a child of T3 Code) there is no such
sandbox, and `docs/reference/limitations-sessions.md` lists those cases. Harness hooks are
the *error message* that names the way out — `ivar feature promote` — never the
barrier. So: supporting a new harness is never blocked on whether it exposes a
pre-tool hook.

## Where things are

The rest of this document lives in `docs/architecture/`, one page per topic:

- **[Module map](docs/architecture/module-map.md)** and its
  **[second part](docs/architecture/module-map-support.md)** — the source tree,
  file by file: entrypoint, `app`, `cli`, `action`, then `domain`, `store`, `git`,
  `providers`, `harness`, `tui`, `infra` and `error.rs`.
- **[Test layout and retained large files](docs/architecture/testing.md)** — where
  tests live and which files are deliberately over the 300-line review trigger.
- **[Layering](docs/architecture/layering.md)** — the table of which layer may
  reach which.
- **[The seams that carry the design](docs/architecture/seams.md)** — seams 1 to 8:
  the action output shape, versioned stores, the canonical writer, `git::Git`, the
  providers facade, the TUI, derived lifecycle (7, 7b, 7c) and CLI conversion.
- **[On-disk layout](docs/architecture/on-disk-layout.md)** — what `ivar` writes
  and where.
- **[Environment contract](docs/architecture/environment-contract.md)** — the
  `IVAR_*` variables, who sets them, and the two scripts with their lifetimes.
- **[Build order and non-goals](docs/architecture/build-order.md)** — the order the
  crate was built in, and what is deliberately not here.
