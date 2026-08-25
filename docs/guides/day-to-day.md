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
projected plan link and the session's instruction file (derived from the
canonical `HALL.md`), all left stale by a promote in another session or by an
older `ivar`:

```sh
eval "$(ivar session connect --feature checkout)"
```

**When a session ends because the model ran out, not because the work did**, relay
it to another provider. The work survives; the conversation does not:

```sh
ivar session relay checkout --provider opencode
```

The relayed session is materialised for the **new** provider (OpenCode's
config and commands — never the old provider's), projects the feature's plan
into its view dir, and derives its instruction file from the hall's canonical
`HALL.md`: the canonical content plus a bootstrap block telling the new agent
to re-derive where the feature is in the SPDD cycle with
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
ivar repo pull --diagnose     # …and, when a branch diverged, show what's on each side
ivar repo pull --resolve      # …and reset branches whose local commits are duplicates upstream
ivar feature rebase checkout  # rebase promoted worktrees onto their defaults
```

`rebase` is best-effort per repo: a dirty worktree is skipped rather than
autostashed, and a conflict aborts that repo and moves on. It will not leave you
half-rebased across five repos.

`repo pull` never rebases or resets a diverged default branch — it reports the
branch as skipped rather than guess. `--diagnose` gives you the local-only and
remote-only commits, so you can tell a branch that simply fell behind from one
with local work that needs reconciling by hand (or that was already re-landed
upstream and is safe to fast-forward). `--resolve` automates that last case: when
every local commit is a duplicate of work already upstream (same patch-id — a
squash, rebase, or cherry-pick), it resets the branch to the remote tip, losing
nothing. It never touches a branch with genuine local work or uncommitted
changes; those are left for you.

## Small changes: skip the artifacts you do not need

Full SPDD is the default, but a gate only exists once its artifact does. A
change with no real design risk — a typo fix, a one-line config change — can
skip Requirements and Analysis and go straight to Plan:

```sh
ivar feature create fix-typo
ivar feature promote fix-typo docs
ivar plan create fix-typo plan       # scaffolds only plan.md
#   ... write plans/fix-typo/plan.md ...
ivar plan approve fix-typo plan      # succeeds: no upstream artifact exists to block it
ivar feature deliver fix-typo --preview
ivar feature deliver fix-typo --fingerprint <fingerprint>
```

This only holds while `requirements.md` and `analysis.md` stay unwritten. The
moment either is written, it blocks `plan approve` exactly as it would in full
SPDD, until it too is approved — the escape is "never written," never
"written and ignored." `ivar plan create fix-typo requirements analysis` is
the upgrade path from here to full SPDD: it writes only the two you are
missing, leaving `plan.md` alone. Reach for the short path only when there is
no real design decision to review; anything riskier earns the full three
artifacts.

## Deliver

Delivery is gated on the **plan** gate. A feature whose plan was never approved
cannot be pushed:

```sh
ivar plan approve checkout plan
```

There is no lifecycle field to set — the gate *is* the state. See
[Planning and execution](planning-and-execution.md) for the gate chain, and
`ARCHITECTURE.md` seam 7 for why it is derived rather than stored.

**Only a root delivers.** A child integrates into its parent instead — see
[Nested subfeatures](../concepts.md#nested-subfeatures) for the leaves-first
flow — and a root with an active, failed, stale, or unintegrated descendant is
blocked until the tree below it is healthy.

Always preview first:

```sh
ivar feature deliver checkout --preview
```

This is side-effect-free. Per promoted repo it shows the branch, the remote, the
refspec, whether a PR would be created or updated, the base branch, and any
blockers — plus the plan gate's state, the tree blockers, and the fingerprint.
Applying runs each repo's ordered checks first; a repo whose checks fail is not
pushed while the rest of the batch continues.

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
