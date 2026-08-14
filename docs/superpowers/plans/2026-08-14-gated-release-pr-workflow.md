# Gated Release PR Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically calculate the next `ivar` version from Conventional Commits, maintain a CI-validated release PR, and publish its `vX.Y.Z` tag only after that PR is merged and the exact merge commit passes `ci` on `main`.

**Architecture:** Keep release policy in `release-plz.toml` and orchestration in the existing `.github/workflows/release-plz.yml`; do not add changeset files or release scripts. Trigger release orchestration from a completed `ci` workflow, reject every event except a successful `push` to `main`, and checkout the upstream run's exact `head_sha`. Run publication before release-PR preparation so registry state is current before the next version is calculated, while `release_always = false` ensures ordinary main commits only prepare a PR and cannot publish.

**Tech Stack:** GitHub Actions `workflow_run`, release-plz `v0.5`, Conventional Commits, Cargo SemVer, crates.io Trusted Publishing (OIDC), GitHub App installation tokens, actionlint.

---

## Resolved behavior

- Conventional Commits are the only change records. Do not introduce Changesets or another version source.
- release-plz maintains one rolling release PR and recalculates it after each successful `main` CI run.
- A human merges the release PR.
- The generated PR must run the normal pull-request CI; use a GitHub App token because PRs created with the default `GITHUB_TOKEN` do not trigger workflows.
- `release-plz release` may publish only a merge commit associated with a `release-plz-*` PR.
- For this single public crate, PRs and tags keep release-plz defaults: `chore: release vX.Y.Z` and `vX.Y.Z`.
- Keep Cargo/release-plz pre-1.0 SemVer behavior: ordinary `feat:` and `fix:` commits increment patch; an incompatible change increments minor. This preserves the documented first release `0.0.1` from `Cargo.toml`.
- Use the repository's existing moving-action convention (`actions/checkout@v5`, etc.); replace the nonexistent `release-plz/action@v0` with the supported `release-plz/action@v0.5` channel.

## File responsibility map

- Modify `release-plz.toml`: define the policy that only merging a release-plz PR authorizes publication.
- Modify `.github/workflows/release-plz.yml`: gate orchestration on the successful upstream CI run, obtain a GitHub App token, publish, and then prepare/update the next release PR.
- No new source, test, workflow, changelog-template, or changeset files.

## Task 1: Provision release identities before merging code

**Files:**
- No repository files.
- GitHub repository settings: GitHub App installation and Actions secrets.
- crates.io package settings: Trusted Publisher for `ivar`.

- [ ] **Step 1: Create the GitHub App**

Create an app named `ivar-release` under the repository owner's GitHub account with:

```text
Webhook: disabled
Repository permissions:
  Contents: Read and write
  Pull requests: Read and write
Installation scope: mnzsss/ivar only
```

The App does not need an Actions permission: events created with its installation token are eligible to trigger the repository's existing workflows.

- [ ] **Step 2: Install the App and create a private key**

Install `ivar-release` on `mnzsss/ivar`, record the numeric App ID, and download one private key. Do not commit either value.

- [ ] **Step 3: Store the GitHub App credentials as Actions secrets**

Create these repository Actions secrets with exact names:

```text
APP_ID=<numeric GitHub App ID>
APP_PRIVATE_KEY=<complete PEM private key, including BEGIN/END lines>
```

Verify only the names, not values:

```bash
rtk gh secret list --app actions
```

Expected: the output contains both `APP_ID` and `APP_PRIVATE_KEY`.

- [ ] **Step 4: Verify the existing crates.io Trusted Publisher**

In the crates.io settings for crate `ivar`, ensure the GitHub Trusted Publisher is:

```text
Owner: mnzsss
Repository: ivar
Workflow filename: release-plz.yml
Environment: unset
```

The crate is already reserved as `0.0.0`, so this is not the unsupported “publish a brand-new crate with Trusted Publishing” case.

## Task 2: Make release-PR merge the publication boundary

**Files:**
- Modify: `release-plz.toml:1-8`

- [ ] **Step 1: Demonstrate that the publication boundary is currently absent**

Run:

```bash
rtk rg '^release_always = false$' release-plz.toml
```

Expected: no match and a non-zero exit status. The current default is `release_always = true`, despite the file comment saying that merging the release PR publishes.

- [ ] **Step 2: Add the minimal release policy**

Make the `[workspace]` section begin exactly as follows, retaining the rest of the file unchanged:

```toml
[workspace]
# release-plz owns versioning: it reads conventional commits, computes the bump,
# generates the CHANGELOG, opens a release PR, tags and publishes on merge.
# Merging the release PR is the act of publishing.
release_always = false
changelog_update = true
git_release_enable = true
git_tag_enable = true
semver_check = true
```

This setting makes `release-plz release` a successful no-op for ordinary `main` commits and permits publication only when the latest commit is associated with a PR whose branch starts with `release-plz-`.

- [ ] **Step 3: Verify the release policy and Cargo metadata**

Run:

```bash
rtk rg -n '^release_always = false$|^git_release_enable = true$|^git_tag_enable = true$|^semver_check = true$' release-plz.toml
rtk cargo metadata --no-deps --format-version 1
```

Expected: four configuration matches; Cargo metadata succeeds and reports package `ivar` at `0.0.0`.

- [ ] **Step 4: Commit the policy separately**

```bash
git add release-plz.toml
git commit -m "fix(release): publish only merged release prs"
```

## Task 3: Gate and modernize the release workflow

**Files:**
- Modify: `.github/workflows/release-plz.yml:1-43`

- [ ] **Step 1: Capture the two current structural failures**

Run:

```bash
rtk rg -n 'push:|release-plz/action@v0$|rust-lang/crates-io-auth-action' .github/workflows/release-plz.yml
```

Expected: matches show that the workflow starts directly on `push`, references the nonexistent `@v0` tag, and obtains a registry token through the now-redundant auth action.

- [ ] **Step 2: Replace the workflow with the gated two-job orchestration**

Replace `.github/workflows/release-plz.yml` with:

```yaml
name: release-plz

on:
  workflow_run:
    workflows: [ci]
    types: [completed]

# workflow_run can cross a privilege boundary, so both jobs repeat a strict gate:
# only the successful push run for main may receive write permissions or OIDC.
jobs:
  release-plz-release:
    name: release-plz release
    if: >-
      ${{
        github.event.workflow_run.conclusion == 'success' &&
        github.event.workflow_run.event == 'push' &&
        github.event.workflow_run.head_branch == 'main'
      }}
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: read
      id-token: write
    steps:
      - name: Generate GitHub App token
        id: app-token
        uses: actions/create-github-app-token@v2
        with:
          app-id: ${{ secrets.APP_ID }}
          private-key: ${{ secrets.APP_PRIVATE_KEY }}

      - name: Checkout validated commit
        uses: actions/checkout@v5
        with:
          ref: ${{ github.event.workflow_run.head_sha }}
          fetch-depth: 0
          persist-credentials: false

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Publish merged release PR
        uses: release-plz/action@v0.5
        with:
          command: release
        env:
          GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}

  release-plz-pr:
    name: release-plz PR
    needs: release-plz-release
    if: >-
      ${{
        github.event.workflow_run.conclusion == 'success' &&
        github.event.workflow_run.event == 'push' &&
        github.event.workflow_run.head_branch == 'main'
      }}
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: write
    concurrency:
      group: release-plz-${{ github.event.workflow_run.head_branch }}
      cancel-in-progress: false
    steps:
      - name: Generate GitHub App token
        id: app-token
        uses: actions/create-github-app-token@v2
        with:
          app-id: ${{ secrets.APP_ID }}
          private-key: ${{ secrets.APP_PRIVATE_KEY }}

      - name: Checkout validated commit
        uses: actions/checkout@v5
        with:
          ref: ${{ github.event.workflow_run.head_sha }}
          fetch-depth: 0
          persist-credentials: false

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Open or update release PR
        uses: release-plz/action@v0.5
        with:
          command: release-pr
        env:
          GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}
```

Do not add `CARGO_REGISTRY_TOKEN`: with `id-token: write`, current release-plz performs the crates.io Trusted Publishing exchange itself. Do not restore `rust-lang/crates-io-auth-action`.

- [ ] **Step 3: Lint the complete workflow**

Run actionlint without adding a repository dependency:

```bash
docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.7
```

Expected: exit status 0 with no diagnostics. If Docker is unavailable, install the same actionlint version outside the repository and run `actionlint`; do not commit a downloaded binary.

- [ ] **Step 4: Verify the security and sequencing invariants mechanically**

Run:

```bash
rtk rg -n "workflows: \[ci\]|types: \[completed\]|conclusion == 'success'|workflow_run.event == 'push'|head_branch == 'main'|workflow_run.head_sha|needs: release-plz-release|release-plz/action@v0.5|id-token: write" .github/workflows/release-plz.yml
! rtk rg 'release-plz/action@v0$|rust-lang/crates-io-auth-action|CARGO_REGISTRY_TOKEN|^[[:space:]]+push:' .github/workflows/release-plz.yml
```

Expected: the first command finds every gate, exact-SHA checkout, ordering edge, supported action channel, and OIDC permission. The negated command exits successfully because the invalid action reference, direct push trigger, external auth action, and long-lived registry token are absent.

- [ ] **Step 5: Run the repository's local CI contract**

Run:

```bash
rtk cargo fmt --all --check
rtk cargo clippy --all-targets --all-features -- -D warnings
rtk cargo test --all-features
```

Expected: all three commands pass. Although the change is YAML/TOML-only, this preserves the same contract enforced by `.github/workflows/ci.yml`.

- [ ] **Step 6: Review the diff and commit the workflow**

Run:

```bash
rtk git diff --check
rtk git diff -- .github/workflows/release-plz.yml release-plz.toml
git add .github/workflows/release-plz.yml
git commit -m "fix(release): wait for successful main ci"
```

Expected: no whitespace errors; the diff contains only the agreed release policy and workflow orchestration.

## Task 4: Prove the live lifecycle after merge

**Files:**
- No additional repository files.
- GitHub Actions, generated release PR, GitHub Releases, and crates.io provide the acceptance evidence.

The `workflow_run` definition must exist on the default branch before GitHub will use it, so its full event behavior cannot be proven from the implementation PR alone. Perform these checks immediately after merging the implementation.

- [ ] **Step 1: Verify the implementation merge's main CI**

```bash
rtk gh run list --workflow ci.yml --branch main --event push --limit 1 \
  --json databaseId,headSha,status,conclusion,url
```

Expected: the latest run targets the implementation merge SHA and concludes `success`.

- [ ] **Step 2: Verify release orchestration waited for that CI**

```bash
rtk gh run list --workflow release-plz.yml --branch main --event workflow_run --limit 1 \
  --json databaseId,headSha,status,conclusion,createdAt,url
```

Expected: one successful release-plz run exists only after the CI run completed. Its publication job succeeds without creating a tag because the implementation merge is not a release-PR merge; its PR job opens or updates the rolling release PR.

- [ ] **Step 3: Inspect the generated release PR**

```bash
rtk gh pr list --state open --search 'head:release-plz-' \
  --json number,title,headRefName,author,url,statusCheckRollup
```

Expected:

```text
title follows: chore: release vX.Y.Z
head branch starts with: release-plz-
author is the configured GitHub App
the PR changes Cargo.toml, Cargo.lock, and CHANGELOG.md
the normal ci checks start and conclude successfully
```

Do not merge until the calculated version and changelog are reviewed and all blocking checks are green.

- [ ] **Step 4: Merge the release PR manually and verify the second gate**

After human approval, merge through the GitHub UI or:

```bash
rtk gh pr merge <RELEASE_PR_NUMBER> --squash
```

Then inspect the newest `ci` and `release-plz` runs with the commands from Steps 1 and 2.

Expected: the release workflow starts only after the release-PR merge commit's `ci` run concludes `success`. The checkout SHA in the release log equals that CI run's `headSha`.

- [ ] **Step 5: Verify publication artifacts agree on one version**

Replace `<VERSION>` with the version reviewed in the PR:

```bash
rtk git fetch --tags
git show-ref --verify "refs/tags/v<VERSION>"
rtk gh release view "v<VERSION>" --json tagName,name,isDraft,isPrerelease,url
cargo search ivar --limit 1
```

Expected:

```text
Git tag: v<VERSION>
GitHub Release: same tag, not draft, not prerelease
crates.io: ivar at <VERSION>
```

- [ ] **Step 6: Prove a failed CI cannot publish**

Use a disposable PR branch that intentionally fails an existing offline CI check (for example, a formatting-only violation), mark it ready for review, and let `ci` conclude `failure`. Do not merge it.

Inspect release runs:

```bash
rtk gh run list --workflow release-plz.yml --event workflow_run --limit 5 \
  --json headSha,status,conclusion,url
```

Expected: GitHub may create a workflow-run record for the completed PR CI, but both privileged jobs are skipped because `workflow_run.event` is `pull_request`; no tag, release, package, or release-PR update is produced. Close the disposable PR afterward.

## Final acceptance checklist

- [ ] `release-plz/action@v0` no longer appears anywhere in `.github/workflows`.
- [ ] `release_always = false` makes release-PR merge the publication boundary.
- [ ] Only a successful `ci` run created by a `push` to `main` can enter a privileged release job.
- [ ] Both jobs checkout `github.event.workflow_run.head_sha`, never the moving tip of `main`.
- [ ] Publication completes before the next release PR is calculated.
- [ ] The GitHub App-authored release PR triggers the existing PR CI.
- [ ] The workflow uses OIDC Trusted Publishing and contains no crates.io token secret.
- [ ] Human approval remains required to merge the release PR.
- [ ] The resulting Cargo version, `vX.Y.Z` tag, GitHub Release, and crates.io version match.
