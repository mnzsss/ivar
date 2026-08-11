# Getting started

## First, what you are getting into

A **hall** is a directory that owns several git repos. The file that defines it,
`ivar.json`, is committed to the hall's own git history — so a hall is something
one person sets up and everyone else clones.

That means there are two ways you arrive here, and they are genuinely different
tasks. Pick the one that describes you:

- **[Someone gave me a hall](#joining-a-hall)** — a repo URL, or a directory that
  already has an `ivar.json`. This is the common case.
- **[I am creating the hall](#creating-a-hall)** — you are the first one in.

If you have not read [Concepts](concepts.md), read it first. It is five terms and
takes about three minutes, and both paths below assume them.

## Before either path: authenticate

`ivar` clones repos and opens pull requests as you. It never stores a credential
of its own; it uses what you already have, in this order:

1. **`gh`**, if it is installed and logged in. `gh auth status` should say so.
2. **`GITHUB_TOKEN`** or **`GH_TOKEN`** in the environment.
3. Otherwise it stops and says so.

There is no anonymous fallback. A private repo failing to clone with an
unhelpful error, twenty minutes into a setup, is worse than being told up front.

```sh
gh auth login      # or: export GITHUB_TOKEN=...
```

## Joining a hall

You have been given a hall's git URL. Clone it and let `ivar` build the rest:

```sh
git clone <hall-url> && cd <hall>
ivar sync
```

`ivar sync` reads the committed `ivar.json` and makes your machine match it:
clones every repo bare, cuts a default-branch worktree for each, materialises
the config for the harnesses the hall uses — including the `/ivar-*` workflow
commands each provider gets — and runs each repo's setup script so a fresh
worktree has its `.env` and its `node_modules`.

That is the onboarding claim, and it is the whole of it — one command, N repos.

Check what you got:

```sh
ivar status
```

Now start working. Either pick up a feature that already exists:

```sh
ivar feature list
ivar session start <feature>
```

…or make one, which is the [day-to-day loop](guides/day-to-day.md).

**`ivar sync` is safe to re-run.** It is idempotent by design: run it whenever
you pull the hall and someone has added a repo. It never touches a git remote —
refreshing branches is `ivar repo pull`, deliberately a separate verb.

## Creating a hall

You are setting up the directory the rest of the team will clone.

```sh
mkdir acme && cd acme
git init
ivar init --name acme
```

That writes three things: `ivar.json` (committed — the hall's identity), `.ivar/`
(local, gitignored — clones, worktrees, state), and the `.gitignore` lines that
keep the second out of the first. It also creates the hall's canonical
instructions (`HALL.md`, committed) and the selected provider's root alias — a
relative symlink (`CLAUDE.md` for Claude Code, `AGENTS.md` for OpenCode) — and
installs the selected provider's `/ivar-*` workflow commands into its native
command directory. Adding a provider later with `ivar provider add` creates
that provider's alias and installs its commands in the same run; `ivar sync`
repairs the whole topology.

Add the repos the team's work spans:

```sh
ivar repo add api  https://github.com/acme/api
ivar repo add web  https://github.com/acme/web
ivar repo add docs https://github.com/acme/docs
```

Each one is declared in `ivar.json`, cloned bare, and given a default-branch
worktree — and its output invites you to run `/ivar-relations <repo>` to record
how it belongs with the other repos in the hall.

If a repo needs bootstrapping that git does not carry — a worktree shares history
but not untracked files, so it has no `.env` and no `node_modules` — give it a
setup script at `.ivar/setups/api.sh`. Write it yourself, or let
`/ivar-repo-setup` inspect the repo and write it for you.

```sh
ivar repo setup api        # runs .ivar/setups/api.sh against api's worktree
```

Commit it. It is part of the hall, and it is what makes your teammates' `ivar
sync` produce a working checkout instead of a bare one.

Then commit and push the hall:

```sh
git add ivar.json .gitignore HALL.md CLAUDE.md AGENTS.md .ivar/setups && git commit -m "hall: api, web, docs"
git push
```

`HALL.md` and the alias symlinks are committed with the hall — a teammate's
`ivar sync` repairs them from there. If a provider's alias ever exists as a
regular file (a legacy hall), `sync` preserves it and asks you to consolidate
its instructions into `HALL.md`; a provider *removed* from `providers.available`
by hand has its alias path deleted on the next sync, even if it is a regular
file.

Everyone else now runs the two commands in [Joining a hall](#joining-a-hall).

## What to read next

- **[Day to day](guides/day-to-day.md)** — the loop: feature → promote → session
  → deliver.
- **[Why not just `git worktree`?](why-not-worktree.md)** — if you are still
  weighing whether this earns its place.
- **[Command reference](reference/commands.md)** — when you know what you want.
