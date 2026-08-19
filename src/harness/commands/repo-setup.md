---
description: Author a repo's Setup Script (.ivar/setups/<repo>.sh) by inspecting the repo and writing the bootstrap steps a fresh worktree needs.
---

# Repo Setup

`/ivar-repo-setup` authors a repo's **Setup Script** —
`.ivar/setups/<repo>.sh` at the hall root — by inspecting the repo and writing
the commands that prepare a freshly-cut worktree (install deps, copy env files,
run codegen). `ivar` runs this script:

- During **`ivar sync`** — against each repo's default-branch worktree.
- On the first **Promote** of that repo — against its new feature-branch
  worktree. Setup failure at promote is **non-fatal**: the repo stays promoted,
  its promotion is recorded as `failed`, and a `feature.setup_script_failed`
  warning names the worktree that was not bootstrapped.

  Note that `ivar repo setup` re-runs the script against the repo's
  **default-branch** worktree only, and so does `ivar sync`. Neither
  re-bootstraps a feature worktree — finish that one by hand, in the worktree
  the warning names.

## Usage

```bash
ivar repo setup                   # no repo → runs every repo's script
ivar repo setup <repo>            # run this repo's script against its worktree
ivar repo setup <repo> --force-setup   # ignore the receipt and run again
```

This workflow drives those commands: inspect the repo, write the script, then
run it.

## When to use

- A repo has no `.ivar/setups/<repo>.sh` yet.
- A teammate asks "make this repo bootstrap itself on promote."
- An existing setup script is stale (build/codegen step changed) and needs
  regenerating.

## Steps

1. **Inspect the repo.** Read the repo's read-only default-branch checkout (or
   its symlink at the current session path's own root, `<repo>/`). Detect, in
   order:
   - **Package manager** from the lockfile: `pnpm-lock.yaml` → `pnpm install`,
     `bun.lockb` → `bun install`, `yarn.lock` → `yarn install`,
     `package-lock.json` → `npm ci`.
   - **Env files**: a committed `.env.example` / `.env.sample` → copy to `.env`
     if absent (`[ -f .env ] || cp .env.example .env`). Never hand-write secret
     values.
   - **Codegen / build prerequisites** in `package.json` scripts (e.g. `prisma
     generate`, `gen:api`, `build` for a library others import) — include only
     steps a worktree genuinely needs before editing, not a full CI run.
   - **Other ecosystems**: `requirements.txt` → `pip install -r`, `Gemfile` →
     `bundle install`, `go.mod` → `go mod download`.
2. **Write the script.** Create `.ivar/setups/<repo>.sh` with the detected
   commands. Keep `set -euo pipefail`. Make every step **idempotent** — the
   user can re-run it, and a partial first run must not wedge the second.
3. **Verify (optional).** If an active feature session has the repo promoted,
   run `ivar repo setup <repo>` and confirm it exits clean.

## Environment available to the script

The script runs via `bash` with `cwd` = the worktree, and these vars set:

| Variable | Description |
|---|---|
| `IVAR_HALL` | The hall root holding `.ivar/` |
| `IVAR_REPO` | Repo name (e.g., `alpha`) |
| `IVAR_BRANCH` | Branch checked out in the worktree |
| `IVAR_WORKTREE` | Absolute path to this worktree (same as cwd) |
| `IVAR_WORKTREE_KIND` | `default` or `feature` |
| `IVAR_FEATURE` | Feature slug (only set when `IVAR_WORKTREE_KIND=feature`) |
| `IVAR_SECRETS_DIR` | `.ivar/secrets/` — gitignored, hand-maintained, never written by `ivar` |

Read secret values from `IVAR_SECRETS_DIR`, never from the script itself:

```sh
[ -f .env ] || cp "${IVAR_SECRETS_DIR}/${IVAR_REPO}.env" .env
```

The directory is local to each machine, so a script that reads from it stays
committable while the values in it never leave the developer's disk.

Because `cwd` is the worktree, plain `pnpm install` / `cp .env.example .env`
land in the right repo.

## Receipt and re-run policy

A successful receipt is stored in the worktree's git administrative directory.
Within the same surviving worktree: absent or invalid receipt → run; same
script fingerprint + prior success → skip; changed fingerprint or prior failure
→ rerun; `--force-setup` → rerun regardless.

## The session hook, when once-per-worktree is wrong

`.ivar/setups/<repo>.session.sh` is a second, optional file for the same repo.
It runs on every `ivar session start`, in the promoted worktree, ungated by any
receipt, and it additionally gets `IVAR_SESSION_ID` and `IVAR_SESSION_PATH`.

Put a step here instead of in the setup script when it must happen once **per
session** rather than once per worktree — bringing up a database or a compose
project that sibling sessions must not share:

```sh
export COMPOSE_PROJECT_NAME="myapp-${IVAR_SESSION_ID}"
docker compose up -d db
```

Everything else belongs in the setup script. A hook that runs `pnpm install`
runs it on every session start, which is the cost the receipt exists to avoid.
A failing hook warns and the session still opens.

## Critical

- **The file is the source of truth** — its presence is what makes setup run.
  There is no manifest field; do not edit `ivar.json`.
- **`.ivar/setups/` is committed and team-shared.** Whatever you write ships to
  every teammate who clones the hall. Keep it generic to the repo, not specific
  to one machine, and never embed secrets.
- **Setup failure at promote is non-fatal** — the repo stays promoted, the
  promotion records `failed`, and a warning is printed. Keep the script safe to
  re-run: nothing re-runs it for you in a feature worktree.
- **Keep it lean.** Bootstrap what a worktree needs to be editable, not a full
  test/CI pipeline.
