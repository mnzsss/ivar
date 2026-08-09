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
   - Each workstream needs: `id`, `title`, `provider` (`claude-code` or
     `opencode`), `depends_on` (ids of workstreams that must finish first),
     `repos` (which repos it touches), `allowed_write_globs` (write contract —
     paths relative to the session view dir), `operations` (list of OP-* IDs
     from the plan that this workstream owns), and optionally `prompt`.
   - **Operation ownership**: Each plan operation must be assigned to exactly
     one workstream. No two workstreams may claim the same operation.
   - Write the candidate graph to a temporary JSON file following this shape:

   ```json
   {
     "version": 1,
     "plan_path": "<plan path from $ARGUMENTS>",
     "workstreams": [
       {
         "id": "api-contract",
         "title": "Define API contracts",
         "provider": "opencode",
         "depends_on": [],
         "repos": ["ecbert"],
         "allowed_write_globs": ["repos/ecbert/apps/ecbert/src/**"],
         "operations": ["OP-API-CONTRACT", "OP-AUTH"],
         "prompt": "Implement the API contracts from the plan"
       },
       {
         "id": "frontend",
         "title": "Update frontend components",
         "provider": "opencode",
         "depends_on": ["api-contract"],
         "repos": ["lagertha"],
         "allowed_write_globs": ["repos/lagertha/src/**"],
         "operations": ["OP-UI"]
       }
     ]
   }
   ```

3. Call `ivar feature execute prepare <feature> --graph-json <path-to-candidate>`.
   This computes the plan fingerprint and validates the graph.

4. **Stop for human approval.** When the board is `AwaitingApproval`, show the
   generated graph to the user and ask them to review. Do not proceed until
   they approve.

5. After approval, call `ivar feature execute approve <feature>`.

6. Call `ivar feature execute tick <feature>` to launch ready workstreams.

7. Monitor progress: check the board status and journal periodically. When
   workstreams finish, re-run `tick` to launch newly-ready workstreams.

8. When a workstream asks a question (blocked), surface the question to the
   user, get their answer, and call `ivar feature execute reply <answer>
   --feature <feature> --session <session>`.

## Important

- **Never hand-edit** `graph.json`, `status.json`, or the journal. Always use
  the CLI commands.
- The `tick` command is idempotent — run it multiple times. It only launches
  workstreams that are pending and whose dependencies have all succeeded.
- Write contracts are enforced by provider guards. If a workstream tries to
  write outside its `allowed_write_globs`, the guard blocks it. Let the user
  know if this happens.
- **Plan fingerprint**: Every graph is pinned to a specific plan revision. If
  the plan changes (behavior-changing), the next `tick` detects the drift and
  pauses affected workstreams. Each paused workstream acknowledges the new
  revision via `ivar feature execute ack-revision <feature> --workstream <id>`.
  Execution resumes only after all affected workstreams acknowledge.
- When all workstreams have succeeded or failed, the execution is complete.
- **Exclusive operations**: Each operation from the plan must belong to exactly
  one workstream. The graph validator enforces this.
