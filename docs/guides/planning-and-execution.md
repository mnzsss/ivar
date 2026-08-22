# Planning and execution

For work too large to hold in one head — or one context window. Three written
artifacts, three approval gates, then provider-coordinated execution.

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
re-derives where the feature is — the gates remain the source of truth.

`plan.md` is written as a **REASONS canvas** — Requirements, Entities, Approach,
Structure, Operations, Norms, Safeguards. The design sections *reference* the
standing sources rather than restating them, and record only this feature's
delta. Its **Operations** section gives the coordinator concrete, testable
implementation steps.

Read them back with `ivar plan show checkout requirements`, and see how far along
every feature is with `ivar plan list`.

## The three gates

```sh
ivar plan approve checkout requirements
ivar plan approve checkout analysis
ivar plan approve checkout plan
```

Each gate is a human decision, in order — a gate refuses until the one upstream
of it is approved. Approving records a fingerprint of the artifact's content.

That fingerprint is what makes the gates mean something: **editing an approved
artifact invalidates it, and cascades downstream.** Change requirements after
approving all three and analysis and plan fall back to needing review. Nothing
silently proceeds on a design that changed underneath it.

To reopen a gate deliberately:

```sh
ivar plan invalidate checkout analysis    # and everything downstream
```

`ivar plan status <path>` reports the gate state of a plan file.

## Execution

After the plan gate is approved, start a provider-coordinated Run Receipt:

```sh
ivar feature execute start checkout
ivar feature execute status checkout
ivar feature execute finish checkout --plan plans/checkout/plan.md \
  --report-json report.json --outcome succeeded
```

The receipt is persistent execution evidence under
`.ivar/features/<feature>/execution/`. It records the approved plan fingerprint,
baseline and finish diff, coordinator identity, and reported outcome. The active
provider coordinates any subagents; Ivar does not schedule them.

### When reality diverges from the plan

If an approved plan changes during a run, `status` reports a diverged receipt.
After the human adopts the revised plan, resume it with:

```sh
ivar feature execute accept-revision checkout --plan plans/checkout/plan.md
ivar feature execute start checkout --resume
```

Use `--restart` to abandon a non-terminal receipt and begin a new one. A terminal
receipt is retained as an archive for audit.

## What this does not do

Starting a receipt does not replace provider coordination. It records the
execution boundary while the chosen provider manages the work.

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
