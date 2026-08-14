# Contributing to ivar

## Open an issue before a pull request

Not bureaucracy — arithmetic. Turning down an issue costs a comment; turning
down a finished pull request costs a person. Describe what you want to change and
why, and wait for a reply before writing code. Small, obvious fixes (typos, a
broken link, a one-line bug with a clear repro) can skip straight to a PR.

**No response-time promise.** This is maintained alongside other work. Issues are
read; some sit for a while.

## What is out of scope

> `ivar` is local-only. It never talks to a server. Anything that requires a
> hosted service is out of scope.

That line is architectural, not commercial, and you can verify it: there is no
network client in this repo, and no telemetry — not even opt-in. A pull request
that adds a server call will be declined on that basis, however good it is.

## The gate

Three commands. All offline, all reproducing exactly what CI runs:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

CI never uses credentials, so no test may touch the network. Fake the boundary
instead — GitHub access goes through a trait, and the fake is what tests see.

Coverage is reported, not enforced. Do not let a coverage number push you into
writing a test you do not believe in.

## Licensing

There is no CLA. Your contribution is licensed under the same dual
`MIT OR Apache-2.0` as the project, and nothing here asks you to assign
copyright.

## If you adapt someone else's code

- **Adapted third-party code into a file?** Put a header at the top of that file
  naming the source, the author, the license and what you changed — and add a
  line to `/NOTICE`. The header travels with the file; a root `NOTICE` does not
  survive a copy-paste.
- **Only learned how something works?** Nothing to do. Ideas are not protected;
  expression is. Reading another project and writing your own version is normal
  engineering, not derivation.

`Cargo.toml` is for dependencies, not for vendored code.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) — `release-plz`
reads them to compute the version bump and write the changelog. A wrong prefix
produces a wrong release.

## Code of Conduct

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).
