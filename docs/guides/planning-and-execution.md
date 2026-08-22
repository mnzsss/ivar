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
committed plan directory, so an agent confined to the session can read and edit
the artifacts, and a relay from one provider to another carries them along.
`ivar plan status plans/<feature>/plan.md` run from inside the session re-derives
where the feature is — the gates remain the source of truth.

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

`ivar plan status <path>` reports the gate state of a plan file. It may also
perform a local-only migration of legacy execution evidence; it never changes a
repository or remote.

## Run an approved plan

After the plan gate is approved, start a Run Receipt from a Feature Session:

```sh
ivar feature execute start checkout --plan plans/checkout/plan.md
```

A Run Receipt is persistent local evidence under
`.ivar/features/<feature>/execution/`. At start, Ivar verifies the feature
session and approved plan fingerprint, records an immutable baseline snapshot,
and attaches the current session and provider to the receipt.

The active provider is the coordinator. It decides how to decompose the approved
plan, create and schedule its native subagents, manage dependencies, and
synthesize results. Ivar does not schedule work, launch headless provider
processes, parse transcripts, or store native-subagent identifiers. Ask a human
directly when a decision needs their input. If newly discovered work lies outside
the approved plan and can be isolated, make it a child Feature rather than
silently expanding this Run.

Check the current receipt at any point:

```sh
ivar feature execute status checkout
ivar feature execute status checkout --history
ivar feature execute status checkout --run <run-id>
```

The default reports the current receipt. `--history` includes archived receipts;
`--run` looks up one current or archived receipt by ID. Use either `--history` or
`--run`, not both.

### States and recovery

| status | meaning | next step |
| --- | --- | --- |
| `active` | A coordinator is attached and work is in flight. | Finish, resume after an interruption, or restart. |
| `blocked` | The coordinator stopped for a human decision. | Resolve it, then resume. |
| `diverged` | The plan fingerprint changed while the Run was active. | Approve and accept the intended revision, then resume. |
| `succeeded` | The coordinator reported success and evidence was recorded. | Inspect history or close the feature when its normal close conditions are met. |
| `failed` | The coordinator reported failure and evidence was recorded. | Inspect history; start a new Run if appropriate. |
| `interrupted` | A non-terminal Run was deliberately replaced or legacy state was imported. | Inspect history; start a new Run if appropriate. |

`active`, `blocked`, and `diverged` are non-terminal. They hold the feature's
single-Run lock, so another coordinator cannot start a competing Run. `succeeded`,
`failed`, and `interrupted` are terminal: the receipt moves to the immutable
archive and releases that lock.

To continue an `active` or `blocked` receipt, attach the current Feature Session:

```sh
ivar feature execute start checkout --plan plans/checkout/plan.md --resume
```

A logical Run can resume with a different provider — for example, begin with
Claude Code and continue with OpenCode. Ivar records the ordered session/provider
lineage, but no provider conversation, transcript, or native-subagent identity
is carried across. The current coordinator must reconstruct its working context
from the approved plan, receipt, repository state, and its current session.

To deliberately end a non-terminal receipt and begin again:

```sh
ivar feature execute start checkout --plan plans/checkout/plan.md --restart
```

The prior receipt becomes `interrupted` and is retained in history.

### Finish with evidence

When the coordinator stops, it writes a structured JSON report and finishes the
receipt:

```sh
ivar feature execute finish checkout --plan plans/checkout/plan.md \
  --report-json /tmp/ivar-run-report.json --outcome succeeded
```

The report must include a non-empty `summary`, at least one task (`title`,
`status`, and `result`), and at least one verification result (`command`,
`status`, and `summary`). It may also record provider-neutral agents,
deviations, blockers, and follow-ups. Do not put provider transcripts or
provider-native identifiers in it.

`--outcome` accepts `succeeded`, `failed`, or `blocked`. A successful or failed
finish atomically records the structured report and final snapshot evidence,
then archives the terminal receipt. A blocked finish preserves the receipt and
its evidence for recovery. Ivar compares the supplied plan to the pinned
fingerprint while finishing: if it changed, the receipt becomes `diverged`
instead of accepting the submitted outcome.

### Accept a changed plan

A changed plan must pass the plan gate again. For a `diverged` receipt, first
confirm that the revised plan remains the intended scope, then accept it:

```sh
ivar plan approve checkout plan
ivar feature execute accept-revision checkout --plan plans/checkout/plan.md
ivar feature execute start checkout --plan plans/checkout/plan.md --resume
```

Accepting a revision records the old and new fingerprints in a checkpoint and
moves the receipt to `blocked`; it does not restart execution by itself.

## History, migration, and close

Receipts are local state, but their archive is durable history: terminal Run
Receipts are never deleted merely because the feature closes. `ivar feature
close` refuses while a non-terminal receipt holds the Run lock, then preserves
both the current execution history and imported legacy evidence.

When opening older local state, Ivar imports a legacy execution record once into
immutable evidence. A legacy completed run keeps its known outcome; any legacy
non-terminal state becomes an `interrupted` receipt because its old scheduling
state cannot be resumed by a provider-native coordinator. The original legacy
record is retained in the archive for audit.

## Repository context

The hall's standing instructions live in the committed `HALL.md`, which also
carries a **Repository relationships** region maintained by `/ivar-relations`:
human-confirmed sentences describing which registered repos belong together.
The planning workflow reads `HALL.md` at the start of Analysis and follows the
linked topics of potentially affected repos; execution and delivery re-check the
context at their start, but `ivar` itself neither parses nor enforces the prose.
