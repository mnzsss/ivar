# Day to day

The loop: **feature → promote → session → deliver.** Four verbs carry a normal
week; everything else is recovery or hygiene.

Assumes you have a hall — see [Getting started](../getting-started.md).

## Start a feature

```sh
ivar feature create checkout
```

A feature is a name plus a branch, with no repos attached yet. The branch is the
feature's name: `checkout` here, in every repo you promote.

Nothing is cloned or cut at this point. Creating a feature is cheap and
reversible, so create one per unit of work rather than reusing a long-lived
branch.

## Promote the repos you will actually change

```sh
ivar feature promote checkout api
ivar feature promote checkout web
```

Each promote refreshes that repo's default branch, cuts a worktree on the
`checkout` branch off its bare clone, runs the repo's setup script if this is the
first time, and makes it writable.

**Promote only what you will edit.** Everything else stays in the session as
read-only default-branch checkouts — visible for context, unwritable by
accident. Promoting later is a single command, so there is no cost to starting
narrow, and there is a real cost to starting wide.

Re-promoting is a no-op. `ivar feature demote checkout web` reverses it; the
worktree stays on disk, so nothing is lost.

## Work

```sh
ivar session start checkout
```

This materialises the view dir, launches the hall's harness in it, and opens a
TUI. Inside, `repos/api` and `repos/web` are writable worktrees on `checkout`;
every other repo is there, read-only.

The one thing worth internalising: **the session holds no state you would miss.**
The branch, the commits, the plan and the promotion records are on disk and in
git. If the agent dies or the process is killed, you have lost a conversation.

Some variations:

```sh
ivar session start checkout --provider opencode   # a specific harness
ivar session start checkout --detached            # no harness; view dir persists
ivar session connect --feature checkout           # re-bind after a restart
```

`session connect` prints the bindings to evaluate in your shell, and repairs the
view dir on the way — symlinks, read-only guards, the provider's config, the
projected plan link and the session's bootstrap instructions, all left stale by
a promote in another session or by an older `ivar`:

```sh
eval "$(ivar session connect --feature checkout)"
```

**When a session ends because the model ran out, not because the work did**, relay
it to another provider. The work survives; the conversation does not:

```sh
ivar session relay checkout --provider opencode
```

The relayed session is materialised for the **new** provider (OpenCode's
config, commands and `AGENTS.md` — never the old provider's), projects the
feature's plan into its view dir, and its bootstrap instructions tell the new
agent to re-derive where the feature is in the SPDD cycle with
`ivar plan status plans/checkout/plan.md` and continue from the first gate
that is `pending` or `needs-revision`. Start the provider in that session's
view dir to pick the work back up.

## See where you are

```sh
ivar feature status checkout   # every promoted repo, and its state
ivar feature list             # all features, and how far each got
ivar status                   # the hall
```

`ivar feature view checkout` opens an interactive dashboard with a live shell per
promoted repo — one terminal each, switched from a sidebar. Useful when you want
to run test suites in three repos side by side.

## Keep up with `main`

```sh
ivar repo pull                # fast-forward every repo's default branch
ivar feature rebase checkout  # rebase promoted worktrees onto their defaults
```

`rebase` is best-effort per repo: a dirty worktree is skipped rather than
autostashed, and a conflict aborts that repo and moves on. It will not leave you
half-rebased across five repos.

## Deliver

Delivery is gated on the **plan** gate. A feature whose plan was never approved
cannot be pushed:

```sh
ivar plan approve checkout plan
```

There is no lifecycle field to set — the gate *is* the state. See
[Planning and execution](planning-and-execution.md) for the gate chain, and
`ARCHITECTURE.md` seam 7 for why it is derived rather than stored.

Always preview first:

```sh
ivar feature deliver checkout --preview
```

This is side-effect-free. Per promoted repo it shows the branch, the remote, the
refspec, whether a PR would be created or updated, the base branch, and any
blockers — plus the plan gate's state and the fingerprint.

Then apply, passing the fingerprint the preview printed:

```sh
ivar feature deliver checkout --fingerprint <fingerprint>
```

The apply is gated on that fingerprint: if anything drifted since you looked, it
refuses rather than pushing something you did not review. Crossing the plan gate
counts as drift too — the gate's state is inside the fingerprinted summary, so a
preview taken before approval cannot be applied after it. Preview again.

Sibling PRs are linked to each other with **`part of`** — never `depends on`.
`ivar` models co-belonging, not dependency: these PRs are parts of one change,
and nothing here claims to know their merge order. The links are added in a
second pass, because a PR's URL does not exist until it has been created.

## Finish

```sh
ivar feature close checkout --outcome delivered   # or: abandoned
```

Close stops executor sessions, removes derived execution state, and records the
outcome in `plan.md`'s frontmatter. The three plan files stay under
`plans/<feature>/`, and the hall's git history is the record.

`ivar feature delete` is the destructive one — worktrees, state, plans. It
preflights write access across the whole cleanup tree and collects every blocker
before touching anything, so a run that cannot finish does not start.

Housekeeping, when features pile up:

```sh
ivar feature prune     # features whose branches were merged
ivar session prune     # sessions no longer bound to anything
ivar cleanup           # stale state; asks before each deletion
```

## The shape of it

```sh
ivar feature create checkout
ivar feature promote checkout api
ivar feature promote checkout web
ivar session start checkout
#   ... work ...
ivar feature deliver checkout --preview
ivar feature deliver checkout
ivar feature close checkout --outcome delivered
```

For work big enough to need a plan before code, see
[Planning and execution](planning-and-execution.md).
