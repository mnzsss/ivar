# Reading the tree: each child's next action

`ivar feature status <parent> --recursive --json` returns `tree`, one entry
per feature in pre-order. Read every child entry (`depth` 1) and take the
first row that matches, top to bottom. Read `plan.md` on disk for the rows
that mention it.

| Entry | Next action |
| --- | --- |
| `state` is `integrated`, `abandoned` or `delivered` | none — done |
| `state` is `failed` or `stale` | run the fix command `ivar feature integrate <child>` names; ask the human when it names none |
| `plan_gate` is `needs_revision` | dispatch a planner with the change, re-review, ask the human to approve again; with a `run` present, `ivar feature execute accept-revision <child>` is the human's call |
| `plan_gate` is `pending`, `plan.md` still the scaffold or a draft | dispatch a planner (step 3) |
| `plan_gate` is `pending`, plan written and reviewed | include it in the batch approval |
| `plan_gate` is `approved`, `repos` empty | promote it once every child it depends on is integrated |
| no `run`, `sessions` empty | `ivar session start <child> --detached --json` |
| no `run`, `sessions` not empty | `ivar feature execute start <child> --mode goal` |
| `run.status` is `active` | `ivar feature execute start <child> --resume`, continue at wave `last_wave` + 1 (wave 1 when `last_wave` is absent) |
| `run.status` is `blocked` | ask the human with the run's blockers (`ivar feature execute status <child>`); resume with `--resume` only on their answer |
| `run.status` is `diverged` | stop and ask the human: the approved plan changed under the run |
| `run.status` is `failed` or `interrupted` | ask the human (step 4) before anything else |
| `run.status` is `succeeded`, `sessions` not empty | `ivar session stop <session-id>` for each id |
| `run.status` terminal, `sessions` empty, `state` is `active` | `ivar feature integrate <child>` |

When every child is done, continue at the final review. A parent entry
(`depth` 0) with an approved plan and a terminal or absent `run` is ready
for it.
