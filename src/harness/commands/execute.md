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

1. Read the plan at `$ARGUMENTS` to understand its structure. Verify the plan's
   approval gate status with `ivar plan status $ARGUMENTS`.

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
     interchangeable names for the same thing.
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

3. Call `ivar feature execute prepare <feature> --graph-json <path-to-candidate>`.
   This computes the plan fingerprint and validates the graph.

4. **Stop for human approval.** When the board is `AwaitingApproval`, show the
   generated graph to the user and ask them to review. Do not proceed until
   they approve.

5. After approval, call `ivar feature execute approve <feature>`.

6. Call `ivar feature execute tick <feature>` to launch every ready
   workstream. `tick` blocks until all of the workstreams it launched have
   terminated — there is nothing to poll while it runs; the call itself is
   the wait.

7. When `tick` returns, check the board status and journal to see how that
   wave landed. If it left other workstreams newly ready (their dependencies
   just succeeded), call `tick` again to launch the next wave.

8. When a workstream asks a question (blocked), surface the question to the
   user, get their answer, and call `ivar feature execute reply <answer>
   --feature <feature> --session <session>`.

## Important

- **Never hand-edit** `graph.json`, `status.json`, or the journal. Always use
  the CLI commands.
- The `tick` command is idempotent — run it multiple times. It only launches
  workstreams that are pending and whose dependencies have all succeeded, and
  each call blocks until the workstreams it launched have terminated before
  returning.
- Write contracts are enforced by provider guards. If a workstream tries to
  write outside its `write_contract`, the guard blocks it. Let the user know
  if this happens.
- **Plan fingerprint**: Every graph is pinned to a specific plan revision. If
  the plan changes (behavior-changing), the next `tick` detects the drift and
  pauses affected workstreams. Each paused workstream acknowledges the new
  revision via `ivar feature execute ack-revision <feature> --workstream <id>`.
  Execution resumes only after all affected workstreams acknowledge.
- When all workstreams have succeeded or failed, the execution is complete.
- **Exclusive operations**: Each operation from the plan must belong to exactly
  one workstream. The graph validator enforces this.
