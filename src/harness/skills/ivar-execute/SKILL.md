---
name: ivar-execute
description: Execute an approved feature plan wave by wave with subagent isolation, lightweight validation, dual-axis review barrier, and gated delivery.
argument-hint: [feature] [plan-path] [--mode goal|default]
---

# ivar-execute

`ivar-execute` executes an approved feature plan wave by wave. It acts strictly as a coordinator: it dispatches implementation to subagents, runs lightweight validation at wave checkpoints, records progress and deferred validation failures in the run receipt with `ivar feature execute checkpoint`, barriers at post-wave dual reviews (Standards review and Spec review), and facilitates human-gated delivery. Never edit `plan.md` during a run: any edit moves its fingerprint and diverges the run.

## Interactive choice convention

For every question with selectable choices:
- If the harness provides an interactive `Ask` tool (or prompt choice mechanism), use `Ask`.
- If `Ask` is unavailable, present the question textually with equivalent lettered options (`a`, `b`, `c`, ...).
- Questions requiring open-ended input remain plain textual prompts. Ask one question per prompt; a choice that approves one step and starts the next ("Approve and continue") is one question.

## Commit and PR text

Never add AI attribution to commit messages, PR titles or PR bodies: no
`Generated with …` line and no `Co-Authored-By:` trailer naming an AI agent.
This overrides any harness default. Pass the rule to every subagent that commits.

## Execution modes

- **`default`**: Guided execution. The human answers at wave gates, validation failures, review findings and delivery, and may let green waves run on without stopping.
- **`goal`**: Autonomous loop from Wave 1 through `execute finish`, fixing failures within the caps, stopping before delivery or integration.

Mode resolution:
1. A new run passes `--mode <mode>` to `ivar feature execute start <feature>` (`default` when omitted).
2. On resume, read the active mode from `ivar feature execute status <feature> --json`. Passing `--mode` with `--resume` records a mode change.
3. The coordinator provider is the last entry in `receipt.coordinators[].provider`.

### Caps (both modes)

| Loop | Cap | When the cap is reached |
| --- | --- | --- |
| Wave lightweight validation | at most 3 fix attempts per wave | Record the failure under `Deferred validation failures` in the wave checkpoint and move on (goal), or ask (default) |
| Post-wave review barrier | at most 3 review-fix rounds | Put the remaining findings in the finish report's `follow_ups` |

### Goal mode

Every provider runs the loop in-skill with the termination condition, caps and stop boundary below. On `claude-code`, the human may also paste this `/goal` line to have the harness hold the coordinator to it:

```text
/goal Run `ivar feature execute status <feature>` and quote its output: the run is `succeeded` or `failed`, and every wave in plan.md has a `wave <n>:` checkpoint. The review barrier ran at most 3 rounds; only hard Standards violations and Spec gaps triggered another round, and whatever remained is in the finish report's follow_ups. No wave took more than 3 validation-fix attempts; anything still failing is listed under Deferred validation failures. `ivar feature deliver` and `ivar feature integrate` were not run. Stop there and hand delivery to the human.
```

Print evidence to the transcript: each wave's validation output and result, each review round's Standards and Spec finding counts, and the final `ivar feature execute status <feature>` output.

## Phase 1: Preparation

1. Resolve the target feature from `$ARGUMENTS` (first positional non-path token), falling back to `$IVAR_FEATURE`. Resolve `<plan-path>` from arguments or default to `.ivar/features/<feature>/plan.md` (resolved from the hall root); parse `--mode <goal|default>`.
2. Run `ivar feature status <feature> --json` and confirm the plan gate is approved; otherwise stop and ask for approval.
3. Every wave in `plan.md` must declare lightweight validation commands (scoped tests, type checks, targeted linters). If one lacks them, stop: the plan must be updated and re-approved.
4. Run `ivar feature execute start <feature> [--plan <plan-path>] [--mode <mode>]`. A run already in progress is resumed per "Recovery" below.

## Phase 2: Waves

Process waves in plan order; never start Wave `K+1` before Wave `K` is checkpointed.

1. **Dispatch:** send the wave's task packets to implementation subagents, each prompt built from `references/subagent.md` with absolute paths and the packet's `**Design ref:**`. The coordinator NEVER writes or edits feature code; subagents make every edit.
2. **Lightweight validation:** run the wave's declared commands. `ivar graph affected <files...>` can point at affected tests; its output is advisory, so fall back to the declared commands or the project's test runner when it is empty or fails.
3. **Green:**
   - Goal mode: print the proof and checkpoint.
   - Default mode: show the result and ask — **(a) Approve and continue** to the next wave, **(b) Approve and run the remaining waves**, stopping only on a failure, **(c) Stop here**.
4. **Red:** up to 3 fix attempts, each dispatched to a subagent and revalidated.
   - Goal mode: after the third, record the failure and checkpoint.
   - Default mode: ask — **(a) Fix now** (within the 3-attempt cap), **(b) Defer failure**: carry it under `Deferred validation failures` into the review.
5. **Checkpoint:**
   ```bash
   ivar feature execute checkpoint <feature> --wave <n> --summary "<completed tasks; exit criteria met; Deferred validation failures: <list or none>>"
   ```
   Never edit `plan.md` during a run — not task checkboxes, not wave notes, not deferred failures. If the plan itself must change, stop and ask the human; re-approval followed by `ivar feature execute accept-revision` is their decision. A request outside the plan is not a plan change: it becomes a child feature (`/ivar-feature-create`).

## Phase 3: Review barrier

1. Once every wave is checkpointed, dispatch two isolated reviewers (concurrently when the harness allows):
   - **Standards review:** repository standards, architecture and baseline smells, per repo.
   - **Spec review:** the implementation against `requirements.md` and `plan.md` across touched repos.
2. Wait for both reports, then present them under separate `## Standards` and `## Spec` headings, with any Deferred validation failures from `ivar feature execute status <feature>`.
3. Repeat the review barrier at most 3 rounds. Only hard Standards violations and Spec gaps block;
   baseline smells are reported once and never re-trigger a round. After round 3, the
   remaining findings go to the finish report's `follow_ups`, where the human sees them at
   the hand-off.
   - Goal mode: dispatch the blocking findings and carried failures to a fix subagent and rerun both reviews. Leftovers go to `follow_ups`; proceed to Phase 4.
   - Default mode: ask once — **(a) Fix all blocking findings and rerun the reviews**, **(b) Fix the findings I select** (no rerun), **(c) Continue to delivery** with the rest in `follow_ups`.

## Phase 4: Finish and hand off

1. Pick the outcome: `succeeded` (validation green, no blocking findings), `failed` (deferred failures or blocking findings left at the cap), or `blocked` (an unrecoverable blocker).
2. Write the report to the session's `.tmp/<feature>-run-report.json`. `ivar feature execute finish --print-schema` prints its shape: `summary`, `tasks` and `verification` are required; `deviations`, `blockers`, `follow_ups` and `agents` are optional. Capped findings go in `follow_ups`.
3. Close the run, then print `ivar feature execute status <feature>` verbatim:
   ```bash
   ivar feature execute finish <feature> --report-json <path> --outcome <succeeded|failed|blocked>
   ```
   Never integrate or deliver while a run is active.
4. Hand off by `is_subfeature` from `ivar feature status <feature> --json`:
   - **Subfeature:**
     - If this session belongs to the parent (orchestrating a child from parent context): return control to the `ivar-subfeatures` loop (which stops the child session and integrates).
     - If this session belongs to the child (`$IVAR_FEATURE` matches the target feature): stop. A child's session ends at `ivar feature execute finish`; the parent session integrates. Print "Stop this session, then integrate from the parent session: `ivar feature integrate <feature> --name "<type>: <short message>"`" (add `--via pr` when configured), filling `--name` with a semantic, squash-ready title for the child's work, as `ivar-deliver` titles PRs.
   - **Root feature driven by `ivar-subfeatures`** (its Wave 0 run on the parent): return control to the `ivar-subfeatures` loop; it creates the children next.
   - **Root feature, goal mode:** stop and print the outcome, the quoted status, and "To deliver: /ivar-deliver". Do NOT run `ivar feature integrate` or `ivar feature deliver`.
   - **Root feature, default mode:** ask — **(a) Draft delivery** (default), **(b) Ready for review**, **(c) Cancel / defer**. Load the `ivar-deliver` skill (`/ivar-deliver`) and follow it end to end: preview with `ivar feature deliver <feature> --preview` (plus `--draft` for Draft delivery), show the preview and fingerprint `<fp>`, and apply with `ivar feature deliver <feature> --fingerprint <fp>` (same `--draft`) once the human confirms.

## Recovery

- **A usage limit or a dead coordinator:** run `ivar feature execute status <feature> --json`, find the last `wave <n>:` checkpoint, run `git status` in each promoted worktree, then `ivar feature execute start <feature> --resume` and continue at wave n+1. A plain `start` refuses with `execute.run_active`.
- **Dead or stuck subagent:** run `git status` and `git diff` in its worktree; keep edits that match the packet, `git stash` the rest, and re-dispatch the same packet with a note on what already landed.
- **Abandon the run:** `ivar feature execute interrupt <feature>`, or `ivar feature execute start <feature> --restart` to start over. Both are the human's call.
- **No live session:** `execute start` needs one. An orchestrating coordinator in a parent session runs `ivar session start <feature> --detached` itself; a standalone child session stops and asks the human to run `/ivar-connect <feature>`.
- **CI fails after deliver:** `gh pr checks <pr>` shows the failing job. Fix on the feature branch (dispatch a subagent), commit, then run `/ivar-deliver` again: a new preview, a new fingerprint, apply. A reviewer's requested changes take the same path.
