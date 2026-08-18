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

There is no CLA, and nothing here asks you to assign copyright. Your
contribution is licensed under the same `Apache-2.0` as the project — which is
what §5 of the license already says about anything you deliberately submit.

What is asked instead is a sign-off. Every commit must carry a `Signed-off-by`
line certifying the [Developer Certificate of Origin](https://developercertificate.org/)
— that you wrote the change, or have the right to submit it:

```sh
git commit -s
```

That is one line in a commit message, not a document to sign, and `-s` writes it
for you. A pull request whose commits lack it will be asked to
`git rebase --signoff` before review.

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

The prefix decides two things. First, the version bump: `feat` moves the minor,
`fix` and `perf` move the patch, and a `!` after the type — or a
`BREAKING CHANGE:` footer — moves the minor while the project is pre-1.0.
Second, where the commit lands in `CHANGELOG.md`:

| Prefix | Changelog section |
| --- | --- |
| `feat` | Added |
| `fix` | Fixed |
| `perf` | Changed |
| `docs` | Documentation |
| any type with `!` | Changed |
| `ci`, `test`, `chore`, `style`, `refactor` | *omitted* |
| anything else | Other |

Those five are omitted because the changelog is read by someone who installed
the binary, and they describe the repository rather than the released artifact.
They are still in git, and they are still reviewed the same way — omission is
not a lower bar. A breaking change is the exception that is never omitted, whatever
its type.

`Other` is not a category to aim for. A commit landing there means its message
did not parse as a conventional commit.

## How a release happens

Nothing is published by hand, and no one pushes a tag.

Every merge to `main` runs CI; when CI passes, `release-plz` opens or updates a
single pull request labelled `release`, titled `chore: release vX.Y.Z`. That PR
holds the version bump and the generated changelog, and it is force-pushed in
place as more commits land — one PR per release cycle, not one per merge.

**Merging that PR is the act of publishing.** It tags the commit, creates the
GitHub Release, publishes to crates.io, and builds the binaries the install
script serves. Do not merge it to "keep it tidy".

If the binary build fails after a release is already out, re-run it alone: the
`release binaries` workflow takes the tag as a `workflow_dispatch` input.

## Code of Conduct

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).
