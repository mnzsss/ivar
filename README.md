# ivar

**Your team already works in a monorepo. It just isn't one repo.**

`ivar` mounts the repos your feature actually spans into one directory — real git
worktrees on the same branch, opened by one agent session. The other team's repo
becomes a folder next to yours. Change a contract in `api`, regenerate the client
in `web`, same session, no handoff.

```sh
curl -fsSL ivar.run/install | sh
```

Or, if you would rather not pipe a script into a shell:

```sh
cargo install ivar                  # builds from source; needs a Rust toolchain
```

Or take a binary straight from a [release](https://github.com/mnzsss/ivar/releases/latest).
Every release carries one per platform — `ivar-linux-x86_64`,
`ivar-linux-aarch64`, `ivar-darwin-x86_64`, `ivar-darwin-aarch64` — each with a
`.sha256` beside it. `chmod +x` it and put it on your `PATH`. These are the same
artifacts the install script above fetches and checksums for you.

> **`ivar` is local-only. It never talks to a server.** Anything that requires a
> hosted service is out of scope. That is a property of the architecture, not a
> policy — read the source and check.

## What it does

- **Halls** — a directory that owns N repos, described by a committed `ivar.json`.
  `git pull && ivar sync` onboards a teammate to every repo at once.
- **Features** — one branch across many repos, with the repos you have not
  promoted held read-only so an agent cannot wander into them.
- **Sessions** — a view dir assembled per feature and opened by your harness of
  choice, with skills and agent config materialised for whichever one you use.
- **Skills** — installed, updated and materialised locally, into each harness's
  native location.
- **Run Receipts** — local, durable records for an approved plan's execution.
  Your provider coordinates native subagents; `ivar` records the plan snapshot,
  evidence, recovery state, and history without storing provider transcripts.

## Status

**Beta**, pre-`0.1.0` — the version you get is whatever the
[latest release](https://github.com/mnzsss/ivar/releases/latest) says, and this
line deliberately does not repeat it. The command surface is settled; the
on-disk format may still move before `0.1.0`. Local state migrates itself;
`ivar.json` never migrates without you asking.

**macOS and Linux.** Windows is not supported: the view dir is built entirely
from symlinks, which need Developer Mode or admin rights on Windows. Use WSL —
it consumes the Linux build with no separate path.

## Documentation

<https://ivar.run>

In this repo: [Concepts](docs/concepts.md) for the model,
[Getting started](docs/getting-started.md) for the two ways in,
[Why not just `git worktree`?](docs/why-not-worktree.md) for the fair objection,
the [planning and execution guide](docs/guides/planning-and-execution.md) for
three-gate SPDD and Run Receipts, and the
[command reference](docs/reference/commands.md). Full index: [docs/](docs/README.md).

## Contributing

Please open an issue before a pull request — see [CONTRIBUTING.md](CONTRIBUTING.md).
Commits must be signed off under the [DCO](https://developercertificate.org/):
`git commit -s`.

## License

Apache License, Version 2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
The licence covers the code, not the name or the marks: see
[TRADEMARK.md](TRADEMARK.md), which is deliberately permissive and lists what
needs no permission at all.

Unless you explicitly state otherwise, any contribution you intentionally submit
for inclusion in this work shall be licensed as above, without any additional
terms or conditions.

---

<sub>single Rust binary · no runtime · no account, no index, no server</sub>
