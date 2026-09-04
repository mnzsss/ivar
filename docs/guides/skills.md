# Skills

A **skill** is a reusable instruction bundle — a folder with a `SKILL.md` — that
the harnesses in your hall can discover. `ivar` keeps the hall's skills in one
place and materialises them into whatever native location each harness expects.

The point is that the skills live in the hall's git history, so the team shares
them by cloning, and nothing hosted is required.

## Where they live

Skills live in one of two roots:

```
<hall>/.ivar/skills/          committed — shared across the team
<hall>/.ivar/skills-local/    personal — gitignored
```

`sync` merges both roots into the harness targets:

```
<hall>/.claude/skills/…
<hall>/.opencode/skills/…
<hall>/.omp/skills/…

`.ivar/skills/` is **committed**, along with `.ivar/setups/` — the hall's
`.gitignore` ignores derived harness targets `.claude/skills/`,
`.opencode/skills/`, and `.omp/skills/`, and includes `.ivar/*` plus `!.ivar/skills/` and `!.ivar/setups/` for
exactly this reason. Everything else under `.ivar/` is derived and ignored.

## Authoring one

```sh
# Defaults to personal root (.ivar/skills-local/)
ivar skill create my-tool --description "..."

# Publishes to shared root (.ivar/skills/)
ivar skill create team-tool --description "..." --hall
```

Personal is the default because writing to the committed root is a commit 
visible in review and inherited by everyone who clones the hall. 

`ivar skill sync` materialises all skills from both roots into
each harness's native target location. Commit `.ivar/skills/` and your
teammates get it on their next `git pull` + `ivar skill sync`.

## Installing someone else's

`ivar skill add` pulls a skill from a GitHub repo. The source can be a shorthand,
a full URL, or a URL pointing straight at one skill:

```sh
ivar skill add acme/skills
ivar skill add https://github.com/acme/skills
ivar skill add https://github.com/acme/skills/tree/main/skills/rust-review
```

When the target holds more than one skill, `add` lists them and asks which to
install; pass `--path` (or the `tree/…/subpath` URL form) to install one without
prompting, and `--ref` to pin a branch, tag or sha. As with `create`, the skill
lands in your personal root unless you pass `--hall`.

```sh
ivar skill add acme/skills --path skills/rust-review --ref v2
ivar skill update            # move external skills to their tracked ref
```

An **external** skill tracks a ref in another repo. `ivar skill update` moves it
forward and re-fetches only that skill's folder; nothing moves on its own, so a
skill cannot change under you between two runs of the same command.

To take ownership of one — stop tracking upstream and keep a local copy you can
edit:

```sh
ivar skill detach rust-review
```

That is one-way, and it is the honest way to fork: the skill becomes authored,
and `update` stops touching it.

## Checking on them

```sh
ivar skill list      # what exists
ivar skill status    # which are external, authored, or stale
ivar skill doctor    # broken links, missing refs, with fixes attached
```

`doctor` returns actionable fixes rather than a diagnosis — each finding carries
the command that resolves it, and whether it is safe to run unattended.

## Removing

```sh
ivar skill remove rust-review
```

## A note on scope

Hall skills apply to **every repo reached in a session**, because they materialise
at the hall root and harnesses find them by walking up from the view dir. That is
the right scope for "how this team works" and the wrong scope for "how this one
repo builds" — repo-specific instructions belong in the repo.

There is no hosted skill sync in `ivar`, and no account. The hall's git repo does
the sharing.

## Not skills: shipped workflow commands

Alongside hall skills, `ivar` ships **workflow commands** — `/ivar-deliver`,
`/ivar-plan`, `/ivar-sync`, and the other official workflows. They are a
separate surface with a separate lifecycle:

```
.ivar/skills/<id>/SKILL.md                 committed hall-owned source
.claude/skills/<id>/...                    derived hall skill target
.opencode/skills/<id>/...                  derived hall skill target
.omp/skills/<id>/...                       derived hall skill target
.claude/commands/ivar-<id>.md              derived Ivar workflow command
.opencode/commands/ivar-<id>.md            derived Ivar workflow command
.omp/commands/ivar-<id>.md                 derived Ivar workflow command
```

Workflow commands are embedded in the binary and materialised by
`ivar init`, `ivar provider add`, and `ivar sync`; they are local derived
state, not team-shared files you edit. The `ivar-*` prefix is reserved for
them — do not name a custom command `/ivar-<something>`, because `ivar sync`
treats anything in that namespace as its own and removes files it did not
ship. A custom command like `/my-cheatsheet` lives happily next to them and is
never touched.
