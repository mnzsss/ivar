# ivar

<p align="center">
  <img src="docs/assets/readme-banner.webp" alt="A stylized Viking longship crossing a Nordic fjord between mountains" width="1200">
</p>

**One logical change can span several repositories. `ivar` gives it one working directory.**

A billing-currency change can touch an API contract in `api`, the generated
client in `web`, and shared types in a third repository. It is one architectural
change; Git sees several repositories. `ivar` keeps the repositories independent
while making that change explicit, safe to scope, and straightforward to resume.

## The model

`ivar` is a local Rust CLI built around five small ideas:

1. **Hall** — a committed directory that declares the repositories a team works
   across in `ivar.json`.
2. **Feature** — one branch name shared by the repositories participating in one
   change.
3. **Promotion** — the explicit decision to make a repository writable on that
   feature. Everything else remains read-only at the filesystem level.
4. **Session** — a view directory opened for you or an agent.
5. **View Dir** — symlinks to the right real worktrees: feature worktrees for
   promoted repositories and guarded default-branch worktrees for the rest.

```text
Hall
  api ── promoted ──► billing-currency worktree (writable)
  web ── promoted ──► billing-currency worktree (writable)
  shared ───────────► main worktree             (read-only)
                     │
                     ▼
          Session / View Dir
            api · web · shared · plans
```

From the view dir, an agent can change the API contract, regenerate the web
client, and update shared types in one session. The plan, branches, and
worktrees remain on disk when the conversation ends.

## Why not move to a monorepo?

A monorepo is a good choice when ownership, access, releases, and tooling belong
together. `ivar` does not ask you to change that repository topology when the
repositories need to stay separate.

It coordinates the existing repositories around a feature instead: one branch
per promoted repository, one view directory, and an explicit writable boundary.
The cross-repository change is coherent without pretending the repositories are
one repository.

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
curl -fsSL ivar.run/install | sh
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

**`ivar` is local-only. It never talks to a server.** It does not run your code,
watch your files, index your repositories, or keep a daemon. It arranges local
directories, worktrees, and provider configuration, then gets out of the way.

It uses your existing GitHub credentials only when it must clone a repository or
open a pull request as you. Read [Concepts](docs/concepts.md) and
[Limitations](docs/reference/limitations.md) for the exact boundaries.

## Status and support

**Beta, pre-`0.1.0`.** The command surface is settled; the on-disk format may
still change before `0.1.0`. Local state migrates itself, while `ivar.json` never
migrates without you asking.

**macOS and Linux.** Windows is not supported because a view dir is built from
symlinks. Use WSL for the Linux build.

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

<sub>single Rust binary · no runtime · no account, no index, no server</sub>
