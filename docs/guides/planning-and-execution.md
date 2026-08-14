# Planning and execution

For work too large to hold in one head — or one context window. Three written
artifacts, four approval gates, then execution across parallel workstreams.

This is optional. A one-repo bug fix does not need it. Reach for it when the
change spans repos and you want the design settled before code starts.

## The three artifacts

```sh
ivar plan create checkout
```

Scaffolds three files under `plans/<feature>/`, **committed** to the hall:

| file | what it holds |
| --- | --- |
| `requirements.md` | what must be true when this is done |
| `analysis.md` | what the code actually looks like now, and the trade-offs |
| `plan.md` | the design, and the concrete operations that implement it |

They are committed on purpose. They are the artifact a reviewer reads and the
record of why the change looks the way it does — so they belong in git history,
not in local state that a `cleanup` could remove.

Inside a feature session, the same directory is projected into the view dir:
`plans/<feature>/` relative to `$IVAR_SESSION_PATH` resolves to the hall's
committed plan directory, so an agent confined to the session can read and
edit the artifacts, and a relay from one provider to another carries them
along. `ivar plan status plans/<feature>/plan.md` run from inside the session
re-derives where the feature is — the gates and the board stay the source of
truth.

`plan.md` is written as a **REASONS canvas** — Requirements, Entities, Approach,
Structure, Operations, Norms, Safeguards. The design sections *reference* the
standing sources rather than restating them, and record only this feature's
delta. Its **Operations** section is the part that matters mechanically: it is
what execution shards into workstreams.

Read them back with `ivar plan show checkout requirements`, and see how far along
every feature is with `ivar plan list`.

## The four gates

```sh
ivar plan approve checkout requirements
ivar plan approve checkout analysis
ivar plan approve checkout plan
ivar feature execute approve checkout     # the fourth gate — see below
```

Each gate is a human decision, in order — a gate refuses until the one upstream
of it is approved. Approving records a fingerprint of the artifact's content.

**The fourth gate is crossed from the execute side, not the plan side.** `ivar
plan approve checkout execution-graph` deliberately refuses and points you at
`ivar feature execute approve`. The graph is board state rather than a document,
and approving it is the act that arms execution — so it belongs to the verb that
owns the board, and having two paths to it would let them disagree about what
"approved" means.

That fingerprint is what makes the gates mean something: **editing an approved
artifact invalidates it, and cascades downstream.** Change requirements after
approving all four and analysis, plan and graph all fall back to needing review.
Nothing silently proceeds on a design that changed underneath it.

To reopen a gate deliberately:

```sh
ivar plan invalidate checkout analysis    # and everything downstream
```

`ivar plan status <path>` reports the gate state of a plan file.

## Execution

Once the graph is approved, the plan runs against a **feature execution board** —
persistent coordination state under `.ivar/features/<feature>/execution/` that
survives sessions. It holds the execution graph, an append-only journal, directed
inboxes per workstream, cursors, blockers and each workstream's write contract.

```sh
ivar feature execute prepare checkout    # build the board from plan + graph
ivar feature execute approve checkout    # awaiting-approval → approved
ivar feature execute tick checkout       # launch whatever is ready
```

`prepare` pins each workstream's `provider`, `model` and `agent` and is
one-shot — a feature that already has a board is refused, because
re-writing it would destroy the journal. Retargeting a workstream therefore means
deleting `board.json` and preparing again from an edited graph, so it is worth
settling who runs what before the board exists rather than after.

A workstream the graph does not target inherits the **caller session's
provider**: `prepare` reads `state.json` from the session named by `--session`
(`/ivar-execute` passes the current one) and resolves that inheritance **before
approval**, so the resolved value is explicit in both `plan.md` and `board.json`
before the board is approved. An explicit per-workstream provider overrides the
session inheritance. `model` and `agent` are separate selectors — they never
choose the provider, only how the chosen provider runs. A board prepared before
provider targeting existed still runs, but `tick` warns
(`execute.legacy_provider_fallback`) that it is falling back to the hall
default rather than hiding the fallback.

`tick` finds workstreams whose dependencies are satisfied and launches them, then
blocks until that wave is over. A wave that leaves work still waiting returns the
board to `approved` — call `tick` again to launch what the wave just unblocked.
When the last workstream finishes the board is `completed`; when one blocks on a
question the board is `blocked` and waits for `reply`.

When a workstream blocks on a question:

```sh
ivar feature execute reply checkout --session <id> --message "use the v2 endpoint"
ivar feature execute tick checkout       # relaunches it with the answer in hand
```

The reply lands in the workstream's inbox and returns it to `waiting`, with the
board back to `approved`. The child that asked the question is gone, so the next
`tick` starts a fresh one — and renders every answer the workstream has been
given into its prompt.

The write contract is enforceable, and a harness can check it before writing:

```sh
ivar feature execute guard-check checkout --session <id> --path api/src/x.rs
```

### When reality diverges from the plan

Two directions, deliberately different verbs:

**The design was wrong** — the approach or the entities do not survive contact,
or the change crosses workstream boundaries. Edit `plan.md`, then:

```sh
ivar feature execute replan checkout --plan plans/checkout/plan.md
```

This advances the plan's fingerprint and **pauses every workstream whose
operations changed** until it acknowledges the new revision. Unaffected
workstreams keep running.

```sh
ivar feature execute ack-revision checkout --workstream api-contract
```

**The code drifted, but only locally** — a different method signature, an
implementation detail confined to one workstream. Record it and keep going:

```sh
ivar feature execute reconcile checkout --workstream api-contract \
  --description "handler returns Result, not Option"
```

Reconcile writes to the journal. It does **not** rewrite the plan.

The distinction is the point: `replan` is design-level, blocking, and happens
before code. `reconcile` is a note that the code and the plan differ in a way
nobody needs to re-approve. Using the first for the second stalls four
workstreams over a type signature.

## What this does not do

**Approving does not run anything.** `ivar feature execute approve` arms the
board; `ivar feature execute tick` is what launches work. Two commands, because
"I approve this design" and "start eight agents now" are different decisions and
should not share a keystroke.

## Repository context

The hall's standing instructions live in the committed `HALL.md`, which also
carries a **Repository relationships** region maintained by `/ivar-relations`:
human-confirmed sentences describing which registered repos belong together.
The planning workflow reads `HALL.md` at the start of Analysis and follows the
linked topics of potentially affected repos; execution and delivery re-check
the context at their own checkpoints. All three checkpoints are evidence-driven
and non-blocking — they only *offer* `/ivar-relations`, they never edit
`HALL.md` themselves, and deferring a review never blocks approval, completion,
or delivery. Rust never parses the relation region; it is prose for humans and
agents.
