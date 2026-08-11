# Repository Relations Journey Specification

**Status:** Ready for implementation planning  
**Date:** 2026-08-11  
**Source map:** [Mapa — Jornada de relações entre repos na hall](https://linear.app/mnzs/issue/MNZS-379/mapa-jornada-de-relacoes-entre-repos-na-hall)

## Goal

When a hall gains a repository, `ivar` invites the human into a provider-neutral
guided journey that records how repositories belong together. The resulting
context is concise, committed, human-authored prose in `HALL.md`; agents read it
while planning, executing, and delivering multi-repository work.

The Rust CLI maintains the canonical instruction file and its provider-native
aliases, but it never parses, validates, infers, or consumes repository
relationships.

## Domain contract

A **Repo relation** is a human-authored directed sentence from one registered
Repo to another registered Repo. It expresses co-belonging: when work starts in
the source Repo, the destination Repo is worth considering too. It does not
assert dependency, permission, build order, merge order, or automatic promotion.

The `/ivar-relations` workflow maintains at most one sentence for each ordered
`(source, destination)` pair. The inverse direction is a separate relation.
Uniqueness and registered names are workflow conventions, not Rust invariants.

This preserves the conceptual part of
[ADR-0002 — Repo relations are directed co-belonging, not dependency](https://linear.app/mnzs/document/adr-0002-repo-relations-are-directed-co-belonging-not-dependency-01f6ea7e371c).
[ADR-0003 — Repo relations live in the canonical hall instructions](https://linear.app/mnzs/document/adr-0003-repo-relations-live-in-the-canonical-hall-instructions-5b8437b0751c)
is the current persistence and consumption decision.

## Trigger and persistence

`ivar repo add` remains atomic and non-interactive. After cloning, creating the
default worktree, and writing `ivar.json`, its successful outcome includes:

```json
{
  "next_action": "/ivar-relations <repo-name>"
}
```

The human rendering prints the same next action after the existing success line.
This is part of a successful `AddOutcome`, not a `Warning`, `Failure`, or
`FixAction`; the exit code remains zero.

The invitation is not persisted. There is no “relations pending” field in
`ivar.json`, no schema version change, no migration, and no `doctor` diagnosis
for an unanswered interview. The CLI does not distinguish “reviewed with no
relations” from “not reviewed”. Ignoring the invitation has no operational
effect. `/ivar-relations` can be opened manually later.

`sync` never starts the interview.

## `/ivar-relations` journey

The shipped provider-neutral command is
`src/harness/commands/relations.md`, materialized as `/ivar-relations`. Its
argument hint is `[repo-name]`.

### Entry modes

- With a Repo argument, evaluate that Repo against every other registered Repo
  in both directions and load every existing relation involving it.
- Without an argument, enter review mode: read all existing relations, ask which
  Repo or ordered pair the human wants to review, and then use the same flow.
- Refuse an unknown Repo name conversationally before proposing changes. The
  workflow reads `ivar.json`; no new Rust relation verb is introduced.

### Evidence gathering

Before asking the human, the agent reads:

1. `ivar.json` for registered names and deterministic Repo order;
2. `HALL.md` for current relation sentences and linked detailed topics;
3. relevant repository manifests, imports, workspace files, generated clients,
   API schemas, automation, and other concrete cross-repository signals.

Evidence may motivate a proposal but can never become a relation without human
confirmation.

### Conversation order

1. Present one proposal at a time.
2. Name the source Repo, destination Repo, concrete evidence, and proposed
   human-readable sentence.
3. Ask the human to **confirm**, **correct**, or **reject** it.
4. A correction is repeated back as the proposed final sentence.
5. After five proposals, ask whether to continue. Five is a soft limit: continue
   only when the human asks to.
6. After code-derived proposals, ask one open question for relationships the
   repositories do not reveal.
7. Show one final summary of additions, sentence replacements, removals, topic
   creations/changes, and newly orphaned topics.
8. Write only after explicit confirmation of that summary.

Rejected proposals are not persisted and may be rediscovered on a later run.
Stopping at the soft limit persists only changes the human confirmed in the
final summary; unreviewed candidates remain unstored.

### Review and mutation rules

- Repeating an ordered pair proposes replacing its sentence; never merge two
  reasons or create duplicate bullets.
- Adding shows the pair and sentence.
- Correcting shows before and after.
- Removing names the pair and current sentence.
- Every mutation requires human confirmation.
- The workflow edits `HALL.md` directly. It never edits `CLAUDE.md`, `AGENTS.md`,
  or a session's provider-native instruction file.
- Before writing, re-read `ivar.json` and `HALL.md` so a concurrent change is
  not overwritten. If the relation region changed since it was read, stop and
  reconcile conversationally.

## Canonical relation region

`/ivar-relations` owns only the bytes between its markers, always outside the
Rust-managed `<!-- ivar:managed:start --> … <!-- ivar:managed:end -->` block:

```markdown
<!-- ivar:relations:start -->
## Repository relationships

### When working in `api`

- Also consider `web`: `web` consumes the REST contract exposed by `api`. [Context](docs/repo-relations/001-api-contract.md)
<!-- ivar:relations:end -->
```

Rules:

- Group by source Repo with `### When working in <repo>`.
- Render one bullet per ordered pair with one short human-authored sentence.
- A context link is optional.
- Order source groups and destination bullets by Repo order in `ivar.json`.
- Normalize after every confirmed mutation and remove empty source groups.
- A review with no relations writes no marker, placeholder, or status.
- Removing the final relation removes the entire region.
- The Rust CLI never owns or rewrites this region.

## Detailed relation topics

When examples, constraints, or history would make the index noisy, the workflow
may propose a user-owned Markdown topic:

```text
docs/repo-relations/001-<slug>.md
```

- Create topics only on demand; never scaffold empty files.
- Use the next `max + 1` three-digit number. Never reuse or renumber identifiers.
- A slug may change with the title if every link is updated in the same confirmed
  mutation.
- One topic may support several related relation bullets.
- The format is a Markdown title and minimal free-form prose; there is no required
  frontmatter or managed block.
- Creating or changing a topic requires human confirmation.
- Removing a relation never deletes its topic automatically.
- When a topic loses its final link, identify it as orphaned and ask whether to
  retain it as history or remove it.

## Canonical hall instructions and aliases

`HALL.md` is the only editable, committed source of standing hall instructions.
It belongs to the user; Rust owns only its existing managed block.

At the hall root:

- `CLAUDE.md` is the Claude Code alias.
- `AGENTS.md` is the OpenCode alias.
- Each enabled provider's alias is a committed relative symlink to `HALL.md`.
- Aliases are never sources and are never workflow edit targets.

`store::layout` must expose separate accessors for the canonical file and a
provider alias. The existing ambiguous provider instruction accessor is replaced
with `hall_instructions()` and `instruction_alias(&Provider)`.

A focused module under `harness/config` is the sole owner of root instruction
inspection and reconciliation. `action` chooses when to invoke it,
`store::layout` computes paths, and `infra::fs` performs atomic I/O. Root alias
symlink operations do not appear in action modules.

## Existing-hall adoption

The canonical state is:

1. regular `HALL.md` containing shared instructions and the Rust-managed block;
2. one relative alias symlink for each enabled provider;
3. no enabled provider alias left as a regular file.

Automatic cases:

- No instruction files: create `HALL.md` and enabled aliases.
- Regular `HALL.md`: update only the managed block, preserving every other byte.
- Missing alias: create it.
- Correct alias: no-op.
- Broken or wrong-target alias for an enabled provider: atomically replace it.

For an enabled provider, a regular `CLAUDE.md` or `AGENTS.md` is never moved,
overwritten, or deleted automatically, even when it is the sole legacy file or
is byte-identical to another file. Instruction reconciliation reports a warning
and `FixAction`; other `sync` work continues.

The human correction checklist is:

1. move a sole legacy file to `HALL.md`, or consciously consolidate divergent
   files into `HALL.md`;
2. remove the enabled provider's old regular alias only after its instructions
   are represented in `HALL.md`;
3. run `ivar sync` to create aliases and the managed block;
4. review the Git diff before committing.

`doctor` reports every incomplete-adoption finding in one pass.

### Destructive disabled-provider rule

When a provider is absent from `providers.available`, its alias path is entirely
Ivar-managed. `sync` removes any entry at that path, including a regular file.
An eventual explicit `provider remove` command has the same behavior. This is a
deliberately destructive exception to enabled-provider adoption safety. It never
removes `HALL.md`.

Because `ivar.json` is hand-edited, manually removing a provider from the
manifest and running `sync` also authorizes this deletion.

## CLI lifecycle matrix

### `ivar init`

After the manifest and hall skeleton are durable, attempt to create `HALL.md`
and the initial provider alias through the shared reconciler. Failure is a
warning and does not roll back the hall; `ivar sync` repairs it.

### `ivar sync`

Authoritatively reconcile the canonical managed block and all provider aliases.
Enabled regular aliases are preserved with an adoption warning. Disabled alias
paths are removed regardless of entry type. Instruction failures do not abort
repo, setup, MCP, or workflow-command reconciliation.

### `ivar provider add`

After the manifest update, immediately reconcile `HALL.md` and the newly enabled
alias through the same module. A conflict is a warning and does not roll back the
provider. `sync` remains the repair.

No provider-removal command is added by this effort. The reconciler's disabled
provider behavior is complete so a future command can call it without defining a
second policy.

### `ivar doctor`

Read-only inspection reports:

- missing or non-regular `HALL.md`;
- missing or stale Rust-managed block;
- missing, regular, broken, or wrong-target enabled aliases;
- any remaining disabled-provider alias entry.

Automatic cases point to `ivar sync`. Enabled regular aliases point to the human
adoption checklist. Disabled aliases warn that `sync` removes the entry,
including a regular file.

### Sessions

Every view dir receives a real, ephemeral provider-native instruction file,
regenerated by comparison in the shared session view materializer:

- discovery session: `HALL.md` content;
- feature session: session bootstrap followed by `HALL.md` content.

Start, connect, conversion, relay, and executor materialization all use this
single path. They read `HALL.md` directly, never the root alias.

If `HALL.md` is absent or non-regular, the session still opens with a warning.
A feature session receives only its bootstrap; a discovery session receives no
shared instruction content. There is no deliberate fallback to a legacy alias.

## Living-context checkpoints

Only provider-neutral Markdown workflows own these reminders. Plan, execution,
and delivery CLI outcomes gain no relation warnings or fix actions.

### `/ivar-plan`

At the beginning of Analysis, always read `HALL.md`, select relations involving
potentially affected Repos, and follow only their linked detailed topics. Use the
context in `analysis.md`. Offer `/ivar-relations` only when concrete code evidence
contradicts, extends, or obsoletes it. Deferring review does not block approval.

### `/ivar-execute`

When every workstream is terminal, inspect the journal and produced changes once.
Offer `/ivar-relations` only with cited evidence of a new, changed, or removed
relation. The choice does not alter execution completion and does not require
replan or reconcile.

### `/ivar-deliver`

Between preview and apply, use preview Repos, Analysis, final journal, and current
relation context for a final check. Offer `/ivar-relations` only for concrete,
unreflected evidence. Deferring it does not block apply or invalidate the
delivery fingerprint.

Evidence includes introduced or removed contracts, generated clients, data
flows, shared packages, automation, or coordinated responsibilities. Mere
co-occurrence in a Feature is not evidence.

## Required shipped-command changes

- Add `src/harness/commands/relations.md`.
- Add `relations` to `harness/commands/catalog.rs`.
- A relations workflow has no Bifrost legacy fingerprint. Change
  `ShippedCommand::legacy_sha256` to `Option<&'static str>`, use `Some(...)` for
  the existing fourteen commands, and `None` for `relations`.
- Materialize `/ivar-relations` for every enabled provider through the existing
  shipped-command reconciliation.
- Add the three living-context checkpoints to `plan.md`, `execute.md`, and
  `deliver.md`.

## Documentation requirements

- Add **Repo relation** to `docs/glossary.md` with the domain contract above and
  a link to **`part of`**.
- Update `ARCHITECTURE.md`, `docs/concepts.md`, and
  `docs/reference/on-disk-format.md` for `HALL.md`, aliases, and session-derived
  files.
- Update user guides where init, sync, providers, and sessions describe
  instruction materialization.
- Reference ADR-0002 only for the surviving conceptual model and ADR-0003 as the
  current persistence/consumption decision.

## Acceptance criteria

1. `repo add` succeeds with a structured `/ivar-relations <repo>` next action.
2. No manifest schema or persistent relation-review state is introduced.
3. `/ivar-relations` follows the one-question-at-a-time hybrid journey and never
   writes without confirmation.
4. The exact relation region is deterministic and disappears when empty.
5. Optional topics follow monotonic three-digit numbering and orphan handling.
6. Rust never parses or rewrites relation prose.
7. `HALL.md` is the sole root source and enabled aliases are relative symlinks.
8. Enabled regular aliases survive `sync`; disabled alias entries do not.
9. Init and provider add attempt immediate materialization; sync repairs;
   doctor diagnoses all drift.
10. Every session derives its native file from `HALL.md`; missing canonical
    content warns but does not block opening.
11. Planning, execution, and delivery reminders are evidence-driven and
    non-blocking.
12. No behavior changes in `feature promote` or delivery fingerprinting.

## Out of scope

- A machine-readable relation graph or manifest schema field.
- Rust validation, querying, or automatic drift detection of relation prose.
- Automatic edits after Repo removal or rename.
- Automatic promotion, promotion suggestions, merge ordering, or delivery
  ordering based on relations.
- Persisted negative proposals or persisted “review complete” state.
- A long-form human onboarding document for repository topology.
- A provider-removal CLI command.
