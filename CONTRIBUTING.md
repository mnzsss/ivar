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
GitHub Release, and publishes to crates.io. Do not merge it to "keep it tidy".

Three things then happen on that tag, in order:

1. `release binaries` builds the four platform binaries the install script
   serves. Each one is smoke-tested before it is uploaded — the right
   architecture, a checksum sidecar in the format `scripts/install.sh` expects,
   and, where the runner can execute what it built, a `--version` that reports
   the version being released.
2. The same workflow then appends an Install section and the SHA-256 of every
   asset to the release body, below release-plz's changelog. It regenerates
   that block rather than appending to it, so re-runs do not stack.
3. `release aur` publishes `ivar` and `ivar-bin` to the Arch User Repository
   from `packaging/aur/`. It is skipped with a warning, not a failure, when
   `AUR_SSH_PRIVATE_KEY` is not configured — see
   [`packaging/aur/README.md`](packaging/aur/README.md).

Both take the tag as a `workflow_dispatch` input, so a step that fails after a
release is already out is re-run alone rather than by cutting a new version:

```sh
gh workflow run release-binaries.yml -f tag=vX.Y.Z
gh workflow run release-aur.yml      -f tag=vX.Y.Z
```

Every third-party action in `.github/workflows/` is pinned to a full commit
SHA with the tag or branch it came from in a trailing comment. A tag is
mutable; the SHA is what actually runs. Move one by resolving the new ref
yourself (`gh api repos/<owner>/<repo>/commits/<ref> --jq .sha`) and updating
the comment in the same edit — never by loosening the pin back to a tag.

## Code of Conduct

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).
