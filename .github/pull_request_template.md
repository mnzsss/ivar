<!--
Small and obvious (typo, broken link, one-line bug with a clear repro)? Send it.
Anything else should have an agreed issue first — see CONTRIBUTING.md.
-->

## What this changes

<!-- One paragraph. What behaviour differs after this lands. -->

Closes #

## Why

<!-- The problem, not the patch. If the issue already says it, link and move on. -->

## Checklist

- [ ] The gate passes locally — all three, all offline:
      `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`
- [ ] Every commit is signed off (`git commit -s`) — CI checks this.
- [ ] Commit messages are [Conventional Commits](https://www.conventionalcommits.org/).
      `release-plz` reads them to compute the version bump; a wrong prefix produces a wrong release.
- [ ] No test touches the network. CI runs without credentials, so a fork's PR
      must pass the same gate — fake the boundary at its trait seam instead.
- [ ] Changed the command surface? `docs/reference/commands.md` is generated
      from the binary, and `tests/docs_reference.rs` fails until it is regenerated.
- [ ] Changed the on-disk format? `docs/reference/on-disk-format.md` states a
      migration promise. Say here how this keeps it.
- [ ] Adapted someone else's code into a file? Header at the top of that file
      naming source, author, licence and what changed — plus a line in `/NOTICE`.
