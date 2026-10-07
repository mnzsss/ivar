# Concepts

`ivar` has five ideas. Everything in the command reference is one of them being
created, changed, or torn down.

The vocabulary is small on purpose, and this page is the whole of it. The
[glossary](glossary.md) has the rest of the terms, and you can reach for it when
a command mentions one — none of them are needed to understand the model.

## The problem, stated once

Your team's work does not fit inside one repo. A change to an API contract lands
in `api`; the client that consumes it lives in `web`; the shared types are in a
third repo. It is one change. Git sees three.

So you open three checkouts, remember which branch each is on, and hope you
noticed when one of them was on `main`. An agent given this situation does worse
than you do, because it cannot see which of the three directories it is allowed
to write to.

## The five terms

### Hall

A directory that owns a set of repos. It is itself a git repo, and the file that
defines it — `ivar.json` — is committed, so a teammate gets your hall by cloning
it and running `ivar sync`.

One hall per scope of work: a team, a product, an initiative. Not one per repo.

### Repo

A git repository the hall knows about, listed in `ivar.json` by name, URL and
default branch.

`ivar` clones it **bare**, once, into `.ivar/repos/<name>/.bare/`. Nothing is
ever checked out into that bare clone. Every working copy — including the one on
the default branch — is a **git worktree** hanging off it.

That is the mechanism the rest of the tool rests on, so it is worth being
concrete: one clone of the objects, many working directories, each on its own
branch, all sharing one history. Two worktrees of the same repo cannot be on the
same branch, which is exactly the safety property that makes them right here.

### Feature

One branch, across however many repos the change touches. It survives sessions,
agents and reboots, because it is a file plus some branches — not a conversation.

A feature starts with no repos attached. You attach them by promoting.

The branch is the feature's name. Create a feature called `checkout` and its
branch is `checkout`, in every repo you promote onto it.

### Promote

Making a repo writable for a feature.

Before promotion, a repo appears in your session on its default branch, and
**it is guarded read-only**. On Linux, for an agent started with
`ivar session start`, Landlock makes the kernel refuse the write: the agent gets
`EACCES`, the same as you would. Elsewhere the guard is a guardrail rather than a
boundary; [Limitations](reference/limitations.md) lists the cases.

Promoting cuts a worktree on the feature's branch off that repo's bare clone,
first refreshing the default branch so the new branch starts from current `main`,
and runs the repo's setup script if it has one. Now it is writable — for every
session bound to that feature, not just this one.

If the feature's branch **already exists**, promotion adopts it: the worktree is
checked out on it as-is, at the commit it already points to. Nothing is rebased
and nothing is reset, because that branch is usually the work you are promoting
for — pushed by a teammate, left behind by a feature you deleted and recreated,
or carried in from whatever you used before `ivar`. Moving it onto current `main`
is `ivar feature rebase`, a separate verb you ask for.

This is the whole of the isolation model: **promoted is writable, everything else
is read-only, and the guarantee is a filesystem one.** Agent hooks are the error
message that names the way out. They are not the barrier, which is why a harness
without hooks is still safe.

### Session

A **view dir** with an agent running in it.

The view dir is a directory of symlinks, one per repo, each pointing at the right
worktree — the feature branch for promoted repos, the shared read-only default
branch for the rest. To the agent, and to you, it is simply a folder where the
other team's repo sits next to yours:

```
.ivar/features/checkout/sessions/<id>/
  api  -> ../../../../repos/api/checkout        (promoted: writable)
  web  -> ../../../../repos/web/checkout        (promoted: writable)
  docs -> ../../../../repos/docs/main           (read-only)
  (working documents live at ../../: requirements.md, analysis.md, plan.md, tasks/)
  CLAUDE.md / AGENTS.md                         (derived from HALL.md: canonical
                                                instructions + session bootstrap
                                                + pointers to each repo's own
                                                CLAUDE.md / AGENTS.md)
```

`cd api`, change the contract, `cd ../web`, regenerate the client. Same
branch, same session, no handoff, nothing pushed in between.

Working documents live directly in the feature directory (`.ivar/features/<feature>/`),
accessible from the session view dir as `../../`, so an agent confined to the
session can read and edit the SPDD artifacts. Three artifacts, three approval gates \u2014 but a
gate exists only once its artifact does. An artifact that was never written is
not a gate, so a change small enough to skip Requirements and Analysis can
carry a `plan.md` alone through to execution; the moment either is written, it
blocks `plan approve`, and voids an approval already granted, until it too is
approved. The hall's standing instructions
live in a single committed `HALL.md`; every view dir — discovery included —
receives its own provider-native instruction file derived from it. A feature
session's file carries the canonical content plus a bootstrap block telling
the agent to re-derive where the feature is with `ivar plan status
../../plan.md` and continue from the first gate that is `pending` or
`needs-revision`; a discovery session's file is the canonical content. Both
end with a generated `## Repository instructions` section that lists, through
the view dir, the absolute path of each linked repo's root instruction file.
The provider root aliases (`CLAUDE.md` / `AGENTS.md` at the hall root) are
relative symlinks to `HALL.md`, never sources. When `HALL.md` is missing, a
session still opens, with a warning and no shared content. The bootstrap block
is what lets a relay from one provider to another pick the work back up.

Each linked repo's **own** instruction files — `CLAUDE.md` / `AGENTS.md` at its
root and in nested directories — reach the agent as it works, and nothing is
copied into the view dir or written inside a repo. `ivar guard` already runs
before every tool call; when it allows a call that touches a path under
`<view>/<repo>/` (a file the tool reads or edits, its working directory, a path
in a shell command), it also returns every instruction file from that repo's
root down to the touched directory that this agent has not received yet, whole
and labelled with its absolute path. Each directory contributes the provider's
own name (`CLAUDE.md` for Claude Code, `AGENTS.md` for OpenCode and OMP), else
the other. A file is delivered once per agent — the main agent and each subagent
separately — and again, marked `UPDATED`, when its content changes, including
when `ivar feature promote` retargets the repo to the feature worktree in the
middle of a session: the view symlink decides which branch is read, with no
restart. A denied call carries nothing, and a failure while computing the
instructions never changes the guard's decision. OpenCode receives the same text
through the plugin `ivar` materialises, appended to the tool's output.

A session is **live** while its view dir exists — liveness is not a process. Kill
the agent, lose the conversation; the branch, the worktrees and the plan are on
disk and in git. That is the property the whole design is arranged around.

## Nested subfeatures

A feature may have a **parent**: create it with `--parent`, and its base is
derived from the parent's branch. Children are unlimited in depth, and a child
always integrates **into its immediate parent's branch** — never an ancestor,
never a default branch — leaves first:

```sh
ivar feature create checkout
ivar feature create checkout-v2
ivar feature create checkout-tax --parent checkout
ivar feature reparent checkout-tax --parent checkout-v2
ivar feature create checkout-tax-ui --parent checkout-tax --via pr
ivar feature integrate checkout-tax-ui
ivar feature integrate checkout-tax
ivar feature deliver checkout-v2 --preview
```

Promote the parent into a repo before any child promotes it: a child
promoted first asks to promote the parent (or refuses with the exact
command), so a child is never silently cut from the default branch. The
shipped `ivar-subfeatures` skill orchestrates the parallel loop with subagents
from the parent session — plans each child, executes goal-mode runs, integrates
leaves as they finish, and prepares the parent delivery preview once.

Each repo's integration is recorded in a **receipt** — source SHA, parent
branch, result SHA, policy, PR URL, and the verification evidence — the moment
it lands. Multi-repo integration is partial and resumable, never atomic: a
successful receipt locks that promotion byte-for-byte, while a failed or
unreceipted one stays repairable. The first receipt of any kind freezes the
child's parent, base, policy, and promotion membership; the fully-integrated
`close` outcome `integrated` freezes the whole child, with no reopen.

Only a **root** delivers. Delivery of a root is blocked by any active, failed,
stale, or unintegrated descendant — abandoned history does not block, but a
descendant beneath an abandoned node still does.

## How they fit

```
ivar.json                     committed: the hall's identity and its repos
  │
  ├── repo  api               one bare clone, many worktrees
  ├── repo  web
  └── repo  docs
        │
        └── feature  checkout          one branch across repos
              ├── promote api          → worktree on `checkout`, writable
              ├── promote web          → worktree on `checkout`, writable
              │   (docs not promoted   → default branch, read-only)
              │
              └── session <uuid>       → view dir of symlinks + an agent
```

## What is deliberately not here

`ivar` does not run your code or hold state in a system daemon, and there is no
ivar server. It arranges directories, keeps a local code graph that never leaves
the machine, and gets out of the way — which is why a session dying costs you a
conversation and nothing else.

It also does not isolate anything but the filesystem and ports. A shared
database stays shared; see [Limitations](reference/limitations.md), which says so
before you find out.

Next: **[Getting started](getting-started.md)**.
