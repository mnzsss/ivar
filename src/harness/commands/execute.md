---
description: Execute a plan as multiple parallel workstreams — decompose, approve, launch, monitor, reply.
argument-hint: <plan-path>
---

# Execute

`/ivar-execute` executes a multi-workstream plan against the current feature.

## Prerequisites

- You must be inside a **Feature Session** (`IVAR_FEATURE` must be set).
- The feature must exist in the hall (`ivar feature list`).
- The plan file must be accessible and its **approval gates** must be passed
  (Requirements → Analysis → Plan all approved).

## Steps

1. Read the plan at `$ARGUMENTS` to understand its structure. Inside a feature
   session the plan is projected into the view dir, so it is reachable at
   `plans/<feature>/plan.md` relative to `$IVAR_SESSION_PATH`. Verify the
   plan's approval gate status with `ivar plan status $ARGUMENTS`.

2. If no Execution Board exists for this feature (no
   `.ivar/features/<feature>/execution/` directory), propose a **candidate
   graph**:

   - Decompose the plan into independent **workstreams** (units of parallel
     work).
   - Each workstream needs: `id`, `title`, `operations` (list of OP-* IDs from
     the plan that this workstream owns), `depends_on` (ids of workstreams
     that must finish first), and `write_contract` (the paths this workstream
     may write, relative to the session view dir). Repos sit at the view
     dir's own root — not under a `repos/` level — so a glob is `<repo>/src/**`,
     never `repos/<repo>/src/**`; the latter matches nothing and the guard
     denies every write without saying why.
   - `provider` (`claude-code` or `opencode`), `model` and `agent` are all
     optional. `model` picks the model the provider runs with; `agent` picks
     which agent definition it runs as — the two are distinct, not
     interchangeable names for the same thing. Omitting a field means the
     provider's own default, which is a fine choice — but it is still a
     choice the user gets to make, in step 3.
   - **Operation ownership**: Each plan operation must be assigned to exactly
     one workstream. No two workstreams may claim the same operation.
   - Write the candidate graph to a temporary JSON file following this shape:

   ```json
   {
     "workstreams": [
       {
         "id": "api-contract",
         "title": "Define API contracts",
         "operations": ["OP-API-CONTRACT", "OP-AUTH"],
         "depends_on": [],
         "write_contract": ["ecbert/apps/ecbert/src/**"],
         "provider": "opencode"
       },
       {
         "id": "frontend",
         "title": "Update frontend components",
         "operations": ["OP-UI"],
         "depends_on": ["api-contract"],
         "write_contract": ["lagertha/src/**"]
       }
     ]
   }
   ```

   This is the whole schema: `id`, `title`, `operations`, `depends_on`,
   `write_contract`, plus the three optional fields above. The graph parser
   denies unknown fields, so `version`, `plan_path`, `repos` and `prompt` are
   all refused — there is no graph schema version and no per-workstream
   prompt to author. The executor's prompt is rendered from the plan itself,
   the workstream's operations and its write contract.

3. **Stop and confirm who runs what.** Before preparing anything, show the
   candidate graph's targeting — one line per workstream with its `provider`,
   `model` and `agent`, writing `—` where the field is unset so the provider
   default is visible rather than implied:

   ```
   api-contract  provider=opencode     model=—              agent=—
   frontend      provider=—            model=—              agent=—
   ```

   Then ask the user whether they want to change the provider, model or agent
   of any workstream. Ask every time, even when every field is defaulted —
   this is the cheap moment. `prepare` is one-shot: once the board exists,
   retargeting a workstream means deleting `board.json` and re-authoring the
   graph, so a question skipped here costs the user the whole setup later.
   Apply their answer to the candidate JSON before continuing.

4. Call `ivar feature execute prepare <feature> --graph-json <path-to-candidate>`.
   This computes the plan fingerprint and validates the graph.

5. **Stop for human approval.** When the board is `AwaitingApproval`, show the
   generated graph to the user — including each workstream's provider, model
   and agent — and ask them to review. Do not proceed until they approve.

6. After approval, call `ivar feature execute approve <feature>`.

7. Call `ivar feature execute tick <feature>` to launch every ready
   workstream. `tick` blocks until all of the workstreams it launched have
   terminated — there is nothing to poll while it runs; the call itself is
   the wait.

8. When `tick` returns, check the board status and journal to see how that
   wave landed. If it left other workstreams newly ready (their dependencies
   just succeeded), call `tick` again to launch the next wave.

9. When a workstream asks a question (blocked), surface the question to the
   user, get their answer, and call `ivar feature execute reply <answer>
   --feature <feature> --session <session>`.

## Important

- **You are the coordinator.** This command is invoked by the coordinator —
  the agent that owns the feature and its tree. When a request arrives that
  falls **outside the approved plan** and is **isolatable**, create the child
  yourself with `ivar feature create <child> --parent <current>`, announce
  it, and do not ask permission. There is **no permission question** before
  child creation. Structural corrections to the approved plan use
  `ivar feature execute replan`; implementation-only local divergence uses
  `ivar feature execute reconcile` — never a silent feature mutation.
- **The executor is not the coordinator.** The executor prompt tells each
  workstream it must never create, reparent, promote, integrate, close,
  delete, or otherwise mutate shared feature state; it stops and reports an
  isolatable request, and you create the child.
- **Never hand-edit** `graph.json`, `status.json`, or the journal. Always use
  the CLI commands.
- **Targeting is pinned at prepare.** There is no command that changes a
  prepared workstream's provider, model or agent — the board would have to be
  deleted and rebuilt. That is why step 3 asks before preparing, and why the
  answer belongs in the candidate JSON rather than in a later correction.
- The `tick` command is idempotent — run it multiple times. It only launches
  workstreams that are pending and whose dependencies have all succeeded, and
  each call blocks until the workstreams it launched have terminated before
  returning.
- Write contracts are enforced at two layers. The provider guard refuses a
  `Write`/`Edit` outside the contract as it happens; a post-run audit compares
  the worktrees against the wave's contracts afterwards, which is what catches
  the writes a shell command made without the guard being asked. The audit
  compares against the commit the run started from, so committing does not
  hide a stray write, and it reads the difference both ways — a run that threw
  away an uncommitted edit it inherited (`git checkout --`, `git reset --hard`,
  `git stash`) is reported too. Either way the workstream is blocked — let the
  user know, and say plainly when reverted content was never committed, since
  it cannot be recovered from the repository.
- A workstream that exits cleanly having changed nothing under its own write
  contract is blocked with a `session.unproductive` journal entry, not marked
  done: there is no work behind it. Surface it to the user rather than
  re-ticking — a second identical run produces the same nothing. The usual
  causes are a prompt the executor could not act on, a `write_contract` that
  does not cover the files the operation actually needs, or a plan operation
  with nothing to implement.
- **Plan fingerprint**: Every graph is pinned to a specific plan revision. If
  the plan changes (behavior-changing), the next `tick` detects the drift and
  pauses affected workstreams. Each paused workstream acknowledges the new
  revision via `ivar feature execute ack-revision <feature> --workstream <id>`.
  Execution resumes only after all affected workstreams acknowledge.
- When all workstreams have succeeded or failed, the execution is complete.
- When every workstream is terminal, inspect the execution journal and the
  produced changes once. Offer `/ivar-relations` only with cited evidence of a
  new, changed, or removed relation. The choice does not alter execution
  completion and is not a replan or reconcile — and this checkpoint never
  writes `HALL.md` directly.
- **Exclusive operations**: Each operation from the plan must belong to exactly
  one workstream. The graph validator enforces this.
