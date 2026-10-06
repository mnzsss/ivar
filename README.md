# ivar

<p align="center">
  <img src="docs/assets/readme-banner.webp" alt="A stylized Viking longship crossing a Nordic fjord between mountains" width="1200">
</p>

**Multi-repo worktrees for coding agents.** One feature across repos, as one unit: one branch, one plan, one pull request per repo, inside the Claude Code, OpenCode or OMP session you already use.

```sh
curl -fsSL https://ivar.run/install | sh
```

Add an `avatarUrl` field to the user profile: the agent describes it in `shared-types`, returns it from `api` and renders it in `web`, with all three mounted as real git worktrees on the `avatar-url` branch. Repos you haven't promoted are guarded (kernel-enforced on Linux; see [Limitations](docs/reference/limitations.md)).

- **Your harness:** Claude Code, OpenCode or OMP, with MCP servers declared once in `ivar.json`.
- **Plan first:** `/ivar-plan` stops for your approval after requirements, analysis and plan.
- **Code graph:** one local SQLite graph of every repo. In a two-repo benchmark, discovery took a median 460k tokens instead of 738k.
- **On your disk:** single Rust binary, Apache-2.0, no account, macOS and Linux. Pull requests go through `gh`.

[Quickstart](https://ivar.run/docs/quickstart) · [Docs](https://ivar.run/docs) · [Why not just git worktree?](docs/why-not-worktree.md)

## The model

`ivar` is a local Rust CLI built around five small ideas:

1. **Hall** — a committed directory that declares the repositories a team works
   across in `ivar.json`.
2. **Feature** — one branch name shared by the repositories participating in one
   change.
3. **Promotion** — the explicit decision to make a repository writable on that
   feature. Everything else stays guarded read-only (kernel-enforced on Linux).
4. **Session** — a view directory opened for you or an agent.
5. **View Dir** — symlinks to the right real worktrees: feature worktrees for
   promoted repositories and guarded default-branch worktrees for the rest.

```text
Hall
  api ────────── promoted ──► avatar-url worktree (writable)
  web ────────── promoted ──► avatar-url worktree (writable)
  shared-types ─ promoted ──► avatar-url worktree (writable)
  infra ────────────────────► main worktree       (read-only)
                             │
                             ▼
                  Session / View Dir
         api · web · shared-types · infra · plans
```

From the view dir, one agent session describes the field in `shared-types`,
returns it from `api` and renders it in `web`. The plan, branches, and
worktrees remain on disk when the conversation ends.

## Why not just `git worktree`?

Use `git worktree` directly when one repository is the problem. `ivar` uses Git
worktrees underneath, then adds the multi-repository parts:

- one committed Hall definition that teammates can clone and `ivar sync`;
- a shared feature branch across the repositories you promote;
- read-only guards for repositories outside the feature's write scope;
- per-repository setup scripts for untracked runtime state; and
- a session view with provider configuration, plans, and durable run receipts.

Read the detailed, candid comparison: [Why not just `git worktree`?](docs/why-not-worktree.md).

## Install

```sh
curl -fsSL https://ivar.run/install | sh
```

The installer defaults to zero telemetry, and the installed `ivar` binary never
sends analytics. Its only unrequested request is the update check: at most once
per 20 hours it asks GitHub for the latest release tag, sending nothing but a
`User-Agent` (turn it off with `IVAR_NO_UPDATE_CHECK=1`). To optionally help
count successful installations:

```sh
curl -fsSL https://ivar.run/install | IVAR_INSTALL_ANALYTICS=1 sh
```

Prefer to build from source?

```sh
cargo install ivar
```

Or download a platform binary from the [latest release](https://github.com/mnzsss/ivar/releases/latest).
Each release includes `ivar-linux-x86_64`, `ivar-linux-aarch64`,
`ivar-darwin-x86_64`, and `ivar-darwin-aarch64`, plus a `.sha256` file for every
artifact. Make the downloaded binary executable and put it on your `PATH`.

## Start with a Hall

If a teammate already has a Hall:

```sh
git clone <hall-url> && cd <hall>
ivar sync
```

If you are creating one:

```sh
mkdir acme && cd acme
git init
ivar init --name acme
ivar repo add api https://github.com/acme/api
ivar repo add web https://github.com/acme/web
```

Then follow the day-to-day loop: create a feature, promote the repositories it
may change, and start a session. The [getting-started guide](docs/getting-started.md)
walks through both paths.

## Local by architecture

**`ivar` is local-only: there is no ivar server, no account, and no analytics in
the binary.** It does not run your code or keep a system daemon. It arranges
local directories, worktrees, and provider configuration, and keeps a local
SQLite code graph of your repositories that never leaves the machine.

The binary makes network requests in four cases, none of them to an ivar server:

- the update check described above;
- `git` and `gh`, with your own credentials, to clone repositories and open pull requests;
- OAuth with the MCP servers your hall declares (`ivar mcp auth`);
- the GitHub API, to fetch external skills.

`ivar upgrade` updates `ivar` through whichever channel installed it. Read
[Concepts](docs/concepts.md) and [Limitations](docs/reference/limitations.md)
for the exact boundaries.

## MCP server

`ivar graph mcp` serves the hall's code graph to your harness over MCP, with the
`graph_explore` and `graph_feedback` tools. It runs inside an ivar hall; start it
with `ivar graph mcp` from the hall root.

mcp-name: io.github.mnzsss/ivar

## Support

**macOS and Linux.** Windows is not supported because a view dir is built from
symlinks. WSL is untested.

Supported agent harness providers: **Claude Code**, **OpenCode**, and **OMP** (Oh My Pi).

## Documentation

- [Concepts](docs/concepts.md) — Hall, Feature, Promotion, Session, and View Dir.
- [Getting started](docs/getting-started.md) — join or create a Hall.
- [Day to day](docs/guides/day-to-day.md) — feature → promote → session → deliver.
- [Planning and execution](docs/guides/planning-and-execution.md) — SPDD and Run Receipts.
- [Command reference](docs/reference/commands.md) — the complete CLI surface.
- [Documentation index](docs/README.md) — everything else.

The hosted documentation is at <https://ivar.run>.

## Contributing

Please open an issue before a pull request — see [CONTRIBUTING.md](CONTRIBUTING.md).
Commits must be signed off under the [DCO](https://developercertificate.org/):
`git commit -s`.

## License

Apache License, Version 2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
The licence covers the code, not the name or the marks: see
[TRADEMARK.md](TRADEMARK.md), which lists what needs no permission.

Unless you explicitly state otherwise, any contribution you intentionally submit
for inclusion in this work shall be licensed as above, without any additional
terms or conditions.

---

<sub>single Rust binary · no runtime · no account · no ivar server</sub>
